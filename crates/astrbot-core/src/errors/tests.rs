//! Tests for astrbot-core error types

#[cfg(test)]
mod tests {
    use astrbot_core::errors::*;

    #[test]
    fn test_error_display() {
        let err = AstrBotError::Config("missing key".to_string());
        assert_eq!(err.to_string(), "configuration error: missing key");

        let err = AstrBotError::Platform {
            adapter: "telegram".to_string(),
            message: "connection refused".to_string(),
        };
        assert_eq!(err.to_string(), "platform error [telegram]: connection refused");
    }

    #[test]
    fn test_event_result_display() {
        assert_eq!(EventResult::Handled.to_string(), "handled");
        assert_eq!(EventResult::Unhandled.to_string(), "unhandled");
        
        let blocked = EventResult::Blocked { reason: "spam".to_string() };
        assert_eq!(blocked.to_string(), "blocked: spam");
        
        let error = EventResult::Error { message: "oops".to_string() };
        assert_eq!(error.to_string(), "error: oops");
    }

    #[test]
    fn test_result_type() {
        let ok_result: Result<i32> = Ok(42);
        assert!(ok_result.is_ok());

        let err_result: Result<i32> = Err(AstrBotError::NotFound("user".to_string()));
        assert!(err_result.is_err());
    }
}
