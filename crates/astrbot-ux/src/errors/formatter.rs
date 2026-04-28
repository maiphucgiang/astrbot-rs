/// 错误消息清理与格式化

/// 去掉机器味前缀
pub fn clean_machine_prefix(msg: &str) -> String {
    let prefixes = [
        "Error: ",
        "error: ",
        "ERROR: ",
        "An error occurred: ",
        "Failed to ",
        "failed to ",
        "thread 'main' panicked at ",
    ];
    
    let mut result = msg.to_string();
    for prefix in &prefixes {
        if result.starts_with(prefix) {
            result = result[prefix.len()..].to_string();
        }
    }
    
    // 去掉常见的栈跟踪标识
    result = result
        .replace("\nStack backtrace:\n", "")
        .replace("\nBacktrace:\n", "");
    
    // 如果清理后为空，返回原始消息
    if result.trim().is_empty() {
        return msg.to_string();
    }
    
    result
}

/// 为 CLI 启动错误添加颜色和结构
#[cfg(feature = "cli-colors")]
pub fn format_cli_error(msg: &str) -> String {
    use colored::*;
    format!("{} {}", "✗".red().bold(), msg.red())
}

#[cfg(not(feature = "cli-colors"))]
pub fn format_cli_error(msg: &str) -> String {
    format!("✗ {}", msg)
}

/// 为聊天场景格式化错误
pub fn format_chat_error(msg: &str) -> String {
    format!("💢 {}", msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_error_prefix() {
        assert_eq!(
            clean_machine_prefix("Error: something bad"),
            "something bad"
        );
        assert_eq!(
            clean_machine_prefix("Failed to connect"),
            "connect"
        );
    }

    #[test]
    fn test_format_cli_error() {
        let result = format_cli_error("测试错误");
        assert!(result.contains("✗"));
        assert!(result.contains("测试错误"));
    }

    #[test]
    fn test_format_chat_error() {
        let result = format_chat_error("测试错误");
        assert!(result.contains("💢"));
        assert!(result.contains("测试错误"));
    }
}
