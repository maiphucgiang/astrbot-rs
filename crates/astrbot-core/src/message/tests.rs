//! Tests for astrbot-core message types

#[cfg(test)]
mod tests {
    use astrbot_core::message::*;
    use astrbot_core::platform::{MessageSource, PlatformType};

    #[test]
    fn test_message_chain_builder() {
        let chain = MessageChain::new()
            .text("Hello ")
            .at("123456")
            .text("!");

        assert_eq!(chain.0.len(), 3);
        assert!(matches!(&chain.0[0], MessageComponent::Plain { text } if text == "Hello "));
        assert!(matches!(&chain.0[1], MessageComponent::At { target, .. } if target == "123456"));
    }

    #[test]
    fn test_plain_text_extraction() {
        let chain = MessageChain::new()
            .text("Hello ")
            .at("123")
            .text(" world");

        assert_eq!(chain.plain_text(), "Hello  world");
    }

    #[test]
    fn test_is_command() {
        let chain = MessageChain::new().text("/help");
        assert!(chain.is_command(&['/']));

        let chain_no_cmd = MessageChain::new().text("hello");
        assert!(!chain_no_cmd.is_command(&['/']));
    }

    #[test]
    fn test_parse_command() {
        let chain = MessageChain::new().text("/echo hello world");
        let result = chain.parse_command(&['/']);
        assert!(result.is_some());

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "echo");
        assert_eq!(args, vec!["hello", "world"]);
    }

    #[test]
    fn test_parse_command_no_args() {
        let chain = MessageChain::new().text("/help");
        let result = chain.parse_command(&['/']);
        assert!(result.is_some());

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "help");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_command_empty_after_prefix() {
        let chain = MessageChain::new().text("/");
        let result = chain.parse_command(&['/']);
        assert!(result.is_none());
    }

    #[test]
    fn test_contains_component() {
        let chain = MessageChain::new()
            .text("Hello")
            .image_url("https://example.com/img.png");

        assert!(chain.contains("plain"));
        assert!(chain.contains("image"));
        assert!(!chain.contains("voice"));
    }

    #[test]
    fn test_message_event_result_reply() {
        let result = MessageEventResult::reply_text("Hello");
        assert!(matches!(result, MessageEventResult::Reply { .. }));
    }

    #[test]
    fn test_message_event_result_nothing() {
        let result = MessageEventResult::nothing();
        assert_eq!(result, MessageEventResult::Nothing);
    }
}
