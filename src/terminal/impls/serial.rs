//! Serial-port session worker (issue #14 / #17).
//!
//! Mirrors the public surface of [`crate::ssh::spawn_session`] so the rest of
//! the UI pipeline (terminal output, key input, tab lifecycle) is reused
//! unchanged: it returns a [`SessionHandle`] plus an
//! [`UnboundedReceiver<SessionEvent>`].
//!
//! Unlike SSH there is no remote PTY, no SFTP and no resource monitor — a
//! serial line is just a raw byte pipe to a switch / router / MCU console.
//! The `serialport` crate is blocking, so the read side runs on a dedicated OS
//! thread and writes go through a dedicated writer thread that is joined on
//! exit so the write handle is always released (#42).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::config::Session;
use crate::i18n::t;
use crate::ssh::{SessionCommand, SessionEvent, SessionHandle};

/// Spawn a serial-port session. See module docs for why the signature mirrors
/// `spawn_session` (minus the PTY size, which a serial line has no notion of).
pub fn spawn_serial_session(
    runtime: &tokio::runtime::Handle,
    tab_id: String,
    session: Session,
) -> (SessionHandle, UnboundedReceiver<SessionEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<SessionCommand>();
    let (evt_tx, evt_rx) = mpsc::unbounded_channel::<SessionEvent>();

    let evt_for_task = evt_tx.clone();
    let join = runtime.spawn(async move {
        if let Err(err) = run_serial(session, cmd_rx, evt_for_task.clone()).await {
            let _ = evt_for_task.send(SessionEvent::Closed(format!("{err:#}")));
        }
    });

    (
        SessionHandle {
            tab_id,
            commands: cmd_tx,
            join,
        },
        evt_rx,
    )
}

fn parse_data_bits(n: u8) -> DataBits {
    match n {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

fn parse_stop_bits(n: u8) -> StopBits {
    match n {
        2 => StopBits::Two,
        _ => StopBits::One,
    }
}

fn parse_parity(s: &str) -> Parity {
    match s {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    }
}

fn parse_flow(s: &str) -> FlowControl {
    match s {
        "hardware" => FlowControl::Hardware,
        "software" => FlowControl::Software,
        _ => FlowControl::None,
    }
}

async fn run_serial(
    session: Session,
    mut commands: UnboundedReceiver<SessionCommand>,
    events: UnboundedSender<SessionEvent>,
) -> Result<()> {
    let port_name = session.serial_port.trim().to_string();
    if port_name.is_empty() {
        return Err(anyhow::anyhow!(t("串口号为空", "serial port is empty")));
    }

    let _ = events.send(SessionEvent::Status(format!(
        "{} {} @ {}",
        t("打开串口", "Opening serial"),
        port_name,
        session.baud_rate
    )));

    // Open on a blocking thread — serialport::open() can stall on a busy
    // device. The timeout bounds the stall: sans it, Close never reaches the
    // command pump (open is awaited before the pump starts) and the task
    // would wedge runtime shutdown forever.
    let open_name = port_name.clone();
    let baud = session.baud_rate;
    let data_bits = parse_data_bits(session.data_bits);
    let stop_bits = parse_stop_bits(session.stop_bits);
    let parity = parse_parity(&session.parity);
    let flow = parse_flow(&session.flow_control);
    let port = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::task::spawn_blocking(move || {
            serialport::new(&open_name, baud)
                .data_bits(data_bits)
                .stop_bits(stop_bits)
                .parity(parity)
                .flow_control(flow)
                // Short read timeout so the reader thread can poll the stop flag.
                .timeout(Duration::from_millis(50))
                .open()
        }),
    )
    .await
    .context(t(
        "打开串口超时",
        "timed out opening the serial port",
    ))?
    .context("serial open task panicked")?
    .with_context(|| {
        format!(
            "{} {}",
            t("打开串口失败", "failed to open serial port"),
            port_name
        )
    })?;

    // A second handle for writing so the reader thread can own the read side.
    let writer = port
        .try_clone()
        .context("failed to clone serial handle for writing")?;

    // A dedicated writer thread owns the write-side handle so a stalled write
    // can never outlive the session: it polls a stop flag between writes and
    // we join it on exit, guaranteeing the handle is released so Windows can
    // reopen the port without "port in use" (#42).
    let writer_stop = Arc::new(AtomicBool::new(false));
    let (write_tx, write_rx) = std::sync::mpsc::channel::<(
        Vec<u8>,
        tokio::sync::oneshot::Sender<std::io::Result<()>>,
    )>();
    let writer_thread = {
        let writer_stop = writer_stop.clone();
        std::thread::spawn(move || {
            let mut writer = writer;
            loop {
                if writer_stop.load(Ordering::Relaxed) {
                    break;
                }
                match write_rx.recv() {
                    Ok((bytes, reply)) => {
                        // No explicit `flush()`: on Unix that maps to `tcdrain`,
                        // which blocks until every byte is physically transmitted
                        // and would hang forever while flow control is stopped.
                        // The driver buffers the bytes and transmits them in order
                        // as the peer drains, which is all a terminal needs (#42).
                        let result = stoppable_write_all(&mut *writer, &bytes, &writer_stop);
                        let _ = reply.send(result);
                    }
                    Err(_) => break, // sender dropped → session closing
                }
            }
        })
    };

    let _ = events.send(SessionEvent::Connected);
    let _ = events.send(SessionEvent::Status(format!(
        "{} {} @ {} {}{}{}",
        t("已连接", "Connected"),
        port_name,
        session.baud_rate,
        session.data_bits,
        parity_letter(&session.parity),
        session.stop_bits,
    )));

    // --- Reader thread ------------------------------------------------------
    let running = Arc::new(AtomicBool::new(true));
    let reader_running = running.clone();
    let reader_events = events.clone();
    let reader_handle = std::thread::spawn(move || {
        let mut port = port;
        let mut buf = [0u8; 4096];
        while reader_running.load(Ordering::Relaxed) {
            match port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if reader_events.send(SessionEvent::Output(text)).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = reader_events.send(SessionEvent::Closed(format!(
                        "{}: {e}",
                        t("串口读取错误", "serial read error")
                    )));
                    break;
                }
            }
        }
    });

    // --- Command pump -------------------------------------------------------
    while let Some(cmd) = commands.recv().await {
        match cmd {
            SessionCommand::RawInput(bytes) => {
                // Never log keystroke bytes — they can be passwords (#15).
                tracing::debug!("serial write len={} bytes", bytes.len());
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if write_tx.send((bytes, reply_tx)).is_err() {
                    let _ = events.send(SessionEvent::Closed(
                        t("串口写入失败", "serial write failed").into(),
                    ));
                    break;
                }
                // Hardware flow control with a stopped peer wedges the write;
                // the writer thread aborts it on the port's write timeout, and
                // this outer bound keeps Close serviceable. If it trips, the
                // writer is marked for stop so its handle is released (#42).
                let res = tokio::time::timeout(Duration::from_secs(30), reply_rx).await;
                match res {
                    Ok(Ok(Ok(()))) => {}
                    Ok(Ok(Err(e))) => {
                        let _ = events.send(SessionEvent::Closed(format!(
                            "{}: {e}",
                            t("串口写入失败", "serial write failed")
                        )));
                        break;
                    }
                    Ok(Err(_)) => {
                        let _ = events.send(SessionEvent::Closed(
                            t("串口写入线程异常", "serial write thread panicked").into(),
                        ));
                        break;
                    }
                    Err(_) => {
                        // The writer is still stalled past the bound; mark it
                        // for stop so it aborts and releases the port handle
                        // promptly, then close (#42).
                        writer_stop.store(true, Ordering::Relaxed);
                        let _ = events.send(SessionEvent::Closed(
                            t("串口写入超时", "serial write timed out").into(),
                        ));
                        break;
                    }
                }
            }
            // A serial line has no window size; nothing to propagate.
            SessionCommand::Resize(_, _) => {}
            SessionCommand::AddTunnel { .. }
            | SessionCommand::StopTunnel(_)
            | SessionCommand::SetResourceMonitoring(_) => {}
            SessionCommand::KillProcess { reply, .. } => {
                let _ = reply.send(crate::ssh::ProcessKillResult {
                    success: false,
                    message: t(
                        "串口不支持远程进程操作",
                        "Remote process control is unavailable for serial sessions",
                    )
                    .into(),
                });
            }
            SessionCommand::Close => break,
        }
    }

    // Stop the writer thread first so the write-side handle is released even
    // if a write is still stalled, then stop the reader (#42).
    writer_stop.store(true, Ordering::Relaxed);
    drop(write_tx);
    let _ = writer_thread.join();

    // Stop the reader thread and wait for it to drain.
    running.store(false, Ordering::Relaxed);
    let _ = reader_handle.join();
    let _ = events.send(SessionEvent::Closed(
        t("串口已关闭", "serial port closed").into(),
    ));
    Ok(())
}

/// `write_all` that aborts as soon as `stop` is set, so the writer thread can
/// be joined (and its serial handle released) even while a peer's flow control
/// has stopped. Each `write` call is bounded by the port's write timeout, so
/// the abort is quick (#42).
fn stoppable_write_all(
    writer: &mut dyn serialport::SerialPort,
    bytes: &[u8],
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        if stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "serial write aborted by close",
            ));
        }
        match writer.write(&bytes[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "serial write returned 0 bytes",
                ))
            }
            Ok(n) => written += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Single-letter parity tag for the status line (8N1 style).
fn parity_letter(parity: &str) -> &'static str {
    match parity {
        "odd" => "O",
        "even" => "E",
        _ => "N",
    }
}
