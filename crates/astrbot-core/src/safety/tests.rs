use crate::message::MessageChain;
use crate::safety::{SafetyEngine, SafetyResult, SafetyStrategy, KeywordFilter, RegexFilter, preset_engine};

#[tokio::test]
async fn test_keyword_filter_safe() {
    let filter = KeywordFilter::new("test", vec!["badword".to_string()], false);
    let chain = MessageChain::new().text("Hello world");
    let result = filter.check(&chain).await;
    assert_eq!(result, SafetyResult::Safe);
}

#[tokio::test]
async fn test_keyword_filter_violation() {
    let filter = KeywordFilter::new("test", vec!["badword".to_string()], false);
    let chain = MessageChain::new().text("This contains badword in text");
    let result = filter.check(&chain).await;
    assert!(matches!(result, SafetyResult::Violation { .. }));
}

#[tokio::test]
async fn test_regex_filter() {
    let mut filter = RegexFilter::new("url_filter");
    filter.add_pattern(r"https?://[^\s]+", "URL detected").unwrap();

    let safe_chain = MessageChain::new().text("Hello world");
    assert_eq!(filter.check(&safe_chain).await, SafetyResult::Safe);

    let bad_chain = MessageChain::new().text("Check out https://example.com");
    let result = filter.check(&bad_chain).await;
    assert!(matches!(result, SafetyResult::Violation { .. }));
}

#[tokio::test]
async fn test_safety_engine_all_safe() {
    let engine = preset_engine();
    let chain = MessageChain::new().text("Hello world");
    let results = engine.check(&chain).await;
    assert!(results.iter().all(|r| matches!(r, SafetyResult::Safe)));
    assert!(engine.is_safe(&chain).await);
    assert!(engine.first_violation(&chain).await.is_none());
}

#[tokio::test]
async fn test_safety_engine_violation() {
    let filter = KeywordFilter::new("spam", vec!["spam".to_string()], false);
    let mut regex = RegexFilter::new("url");
    regex.add_pattern(r"https?://", "link").unwrap();

    let engine = SafetyEngine::new()
        .add_strategy(Box::new(filter))
        .add_strategy(Box::new(regex));

    let chain = MessageChain::new().text("Buy now spam https://bad.com");
    let results = engine.check(&chain).await;
    assert!(!engine.is_safe(&chain).await);
    assert_eq!(results.len(), 1); // stop_on_first = true

    let violation = engine.first_violation(&chain).await;
    assert!(violation.is_some());
}

#[tokio::test]
async fn test_safety_engine_collect_all() {
    let filter1 = KeywordFilter::new("spam", vec!["spam".to_string()], false);
    let mut filter2 = RegexFilter::new("url");
    filter2.add_pattern(r"https?://", "link").unwrap();

    let engine = SafetyEngine::new()
        .with_stop_on_first(false)
        .add_strategy(Box::new(filter1))
        .add_strategy(Box::new(filter2));

    let chain = MessageChain::new().text("Buy spam now https://bad.com");
    let results = engine.check(&chain).await;
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|r| matches!(r, SafetyResult::Violation { .. })));
}

#[tokio::test]
async fn test_keyword_filter_case_sensitive() {
    let filter = KeywordFilter::new("case", vec!["Bad".to_string()], true);
    let chain = MessageChain::new().text("This is bad");
    assert_eq!(filter.check(&chain).await, SafetyResult::Safe);

    let chain2 = MessageChain::new().text("This is Bad");
    assert!(matches!(filter.check(&chain2).await, SafetyResult::Violation { .. }));
}
