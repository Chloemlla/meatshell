#[path = "../src/allocator.rs"]
mod allocator;

use allocator::allocator_name;

#[cfg(test)]
mod tests {
    use super::allocator_name;

    #[cfg(target_os = "windows")]
    const EXPECTED: &str = "mimalloc";

    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    const EXPECTED: &str = "jemalloc";

    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    const EXPECTED: &str = "system";

    #[test]
    fn selects_allocator_for_target_platform() {
        assert_eq!(allocator_name(), EXPECTED);
    }

    #[test]
    fn allocates_and_releases_memory() {
        let mut values = Vec::with_capacity(1024);
        values.extend(0..1024);
        assert_eq!(values.len() * std::mem::size_of::<i32>(), 4096);
        assert_eq!(values[0], 0);
        assert_eq!(values[1023], 1023);
    }
}

fn main() {
    let mut values = Vec::with_capacity(1024);
    values.extend(0..1024);
    println!("allocator={}", allocator_name());
    println!(
        "allocated_bytes={}",
        values.len() * std::mem::size_of::<i32>()
    );
}
