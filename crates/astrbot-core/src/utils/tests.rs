//! Tests for astrbot-core utilities

#[cfg(test)]
mod tests {
    use astrbot_core::utils::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("hello/world"), "hello_world");
        assert_eq!(sanitize_filename("file:name*?"), "file_name__");
        assert_eq!(sanitize_filename("normal.txt"), "normal.txt");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world this is long", 10), "hello w...");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate("hello world", 11), "hello world");
    }
}
