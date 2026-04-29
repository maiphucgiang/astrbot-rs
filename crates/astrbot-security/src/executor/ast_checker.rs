use anyhow::{bail, Result};
use serde_json::Value;

/// 白名单模块：只允许这些 import
static ALLOWED_MODULES: &[&str] = &[
    "json",
    "re",
    "math",
    "random",
    "datetime",
    "statistics",
    "hashlib",
    "hmac",
    "base64",
    "string",
    "collections",
    "itertools",
    "functools",
    "operator",
    "types",
    "typing",
    "dataclasses",
];

/// 黑名单 AST 节点：检测到直接拒绝
static FORBIDDEN_NAMES: &[&str] = &[
    "__import__",
    "eval",
    "exec",
    "compile",
    "open",
    "os",
    "sys",
    "subprocess",
    "socket",
    "urllib",
    "http",
    "ctypes",
    "ffi",
    "platform",
    "pwd",
    "grp",
    "shutil",
    "pathlib",
    "tempfile",
    "multiprocessing",
    "threading",
];

pub struct AstChecker;

impl AstChecker {
    /// 调用 Python 的 ast 模块做解析
    pub fn check(python_code: &str) -> Result<()> {
        let forbidden_json = serde_json::to_string(FORBIDDEN_NAMES)?;
        let allowed_json = serde_json::to_string(ALLOWED_MODULES)?;

        let script = format!(
            r#"
import ast, json, sys
code = sys.argv[1]
try:
    tree = ast.parse(code)
except SyntaxError as e:
    print(json.dumps({{"ok": false, "error": "SyntaxError: " + str(e)}}))
    sys.exit(0)

forbidden = {forbidden}
allowed = {allowed}
issues = []

for node in ast.walk(tree):
    if isinstance(node, ast.Import):
        for alias in node.names:
            mod = alias.name.split('.')[0]
            if mod not in allowed:
                issues.append("Forbidden import: " + mod)
    elif isinstance(node, ast.ImportFrom):
        mod = node.module.split('.')[0] if node.module else ''
        if mod not in allowed:
            issues.append("Forbidden import from: " + mod)
    elif isinstance(node, ast.Call):
        if isinstance(node.func, ast.Name) and node.func.id in forbidden:
            issues.append("Forbidden call: " + node.func.id)
        elif isinstance(node.func, ast.Attribute):
            chain = []
            n = node.func
            while isinstance(n, ast.Attribute):
                chain.append(n.attr)
                n = n.value
            if isinstance(n, ast.Name):
                chain.append(n.id)
            chain.reverse()
            name = '.'.join(chain[:2])
            if any(f in name for f in forbidden):
                issues.append("Forbidden attribute chain: " + name)
    elif isinstance(node, ast.Subscript):
        if isinstance(node.value, ast.Name) and node.value.id == '__builtins__':
            issues.append("Forbidden: __builtins__ access")

print(json.dumps({{"ok": len(issues) == 0, "issues": issues}}))
"#,
            forbidden = forbidden_json,
            allowed = allowed_json,
        );

        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(&script)
            .arg(python_code)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: Value = serde_json::from_str(&stdout).map_err(|e| {
            anyhow::anyhow!("AST check JSON parse failed: {} (stdout: {})", e, stdout)
        })?;

        if let Some(false) = result["ok"].as_bool() {
            let issues = result["issues"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|v| v.as_str().unwrap_or("?"))
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_else(|| "Unknown AST violation".to_string());
            bail!("CodeExecutor AST check failed: {}", issues);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_code() {
        let code = r#"
import json, math
result = json.dumps({"x": math.sqrt(4)})
"#;
        assert!(AstChecker::check(code).is_ok());
    }

    #[test]
    fn test_forbidden_import() {
        let code = "import os\nos.system('id')";
        assert!(AstChecker::check(code).is_err());
    }

    #[test]
    fn test_eval_call() {
        let code = "eval('1+1')";
        assert!(AstChecker::check(code).is_err());
    }

    #[test]
    fn test_builtins_bypass() {
        let code = "__builtins__['__import__']('os')";
        assert!(AstChecker::check(code).is_err());
    }
}
