/// 错误消息对照表
/// 将常见机器错误映射为人类可读文案

use std::collections::HashMap;
use lazy_static::lazy_static;

lazy_static! {
    static ref ERROR_CATALOG: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        
        // 连接类
        m.insert("Connection refused", "连不上服务，检查一下地址和端口对不对，或者服务有没有启动。");
        m.insert("Connection timed out", "连接超时了，可能是网络不稳定，或者对方服务太忙。");
        m.insert("Timeout after", "等太久了没回应，可能是网络抽风或者服务商忙。再试一次？");
        m.insert("dns error", "域名解析失败，检查一下网络连接，或者地址是否写对了。");
        
        // 认证类
        m.insert("Invalid API key", "API Key 看起来不对，检查有没有多余的空格，或者是不是过期了。");
        m.insert("Unauthorized", "认证失败了，检查一下 token 或 API key 是否有效。");
        m.insert("Permission denied", "这个操作需要管理员权限。如果你就是管理员，检查 config 里的 admin_id 设置。");
        
        // 配置类
        m.insert("config file not found", "找不到配置文件。第一次运行？让我帮你初始化一个。");
        m.insert("missing field", "配置里少了一项必填内容，对照文档检查一下。");
        m.insert("Plugin not found", "插件没找到。可能是名字打错了，或者还没安装。用 /plugin list 看看有哪些可用的。");
        
        // 限流类
        m.insert("Rate limit", "请求太快了，服务商限流了。等一分钟再试，或者换个模型。");
        m.insert("Too many requests", "请求太频繁，被限制了。稍等片刻再试。");
        
        // 模型类
        m.insert("model not found", "这个模型不可用，可能是名字打错了，或者服务商不支持。用 /model list 看看有哪些可用的。");
        m.insert("context length exceeded", "对话太长了，超过了模型的上下文限制。用 /reset 重置一下记忆。");
        
        m
    };
}

/// 尝试从错误消息中匹配已知错误
pub fn match_known_error(err: &dyn std::error::Error) -> Option<String> {
    let msg = err.to_string();
    
    for (pattern, human_msg) in ERROR_CATALOG.iter() {
        if msg.contains(pattern) {
            return Some(human_msg.to_string());
        }
    }
    
    // 递归检查 source
    if let Some(source) = err.source() {
        return match_known_error(source);
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_connection_refused() {
        let err = std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "Connection refused (os error 111)"
        );
        let result = match_known_error(&err);
        assert!(result.is_some());
        assert!(result.unwrap().contains("连不上服务"));
    }

    #[test]
    fn test_match_api_key() {
        let err = std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid API key: sk-xxx"
        );
        let result = match_known_error(&err);
        assert!(result.is_some());
        assert!(result.unwrap().contains("API Key"));
    }

    #[test]
    fn test_no_match_returns_none() {
        let err = std::io::Error::new(
            std::io::ErrorKind::Other,
            "Some completely unknown error xyz123"
        );
        let result = match_known_error(&err);
        assert!(result.is_none());
    }
}
