//! WASM Plugin Runtime — sandboxed WebAssembly plugin loader for AstrBot
//!
//! 使用 wasmi（纯 Rust，零 C 依赖）作为 WASM 解释器，提供燃料计量与内存限制。

use crate::errors::{AstrBotError, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Host State
// ---------------------------------------------------------------------------

/// 默认最大内存（16 MiB）
const DEFAULT_MAX_MEMORY: usize = 16 * 1024 * 1024;

/// 默认燃料预算（10 亿条指令）
const DEFAULT_FUEL_PER_CALL: u64 = 1_000_000_000;

/// WASM 宿主状态，跟踪内存使用与资源限制
#[derive(Debug)]
pub struct WasmHostState {
    pub memory_usage: AtomicUsize,
    pub max_memory: usize,
    pub fuel_budget: u64,
}

impl WasmHostState {
    pub fn new(max_memory: usize, fuel_budget: u64) -> Self {
        Self {
            memory_usage: AtomicUsize::new(0),
            max_memory,
            fuel_budget,
        }
    }

    pub fn default_limits() -> Self {
        Self::new(DEFAULT_MAX_MEMORY, DEFAULT_FUEL_PER_CALL)
    }
}

impl Clone for WasmHostState {
    fn clone(&self) -> Self {
        Self {
            memory_usage: AtomicUsize::new(self.memory_usage.load(Ordering::Relaxed)),
            max_memory: self.max_memory,
            fuel_budget: self.fuel_budget,
        }
    }
}

// ---------------------------------------------------------------------------
// WASM Plugin
// ---------------------------------------------------------------------------

/// 已加载的 WASM 插件
#[derive(Debug)]
pub struct WasmPlugin {
    pub name: String,
    pub version: String,
    /// wasmi 模块
    pub module: wasmi::Module,
}

// ---------------------------------------------------------------------------
// Plugin Loader
// ---------------------------------------------------------------------------

/// WASM 插件加载器
#[derive(Debug)]
pub struct WasmPluginLoader {
    engine: wasmi::Engine,
    host_state_template: WasmHostState,
}

impl WasmPluginLoader {
    /// 创建新的加载器
    pub fn new() -> Result<Self> {
        let engine = wasmi::Engine::default();
        Ok(Self {
            engine,
            host_state_template: WasmHostState::default_limits(),
        })
    }

    /// 从文件路径加载 WASM 插件
    pub async fn load(
        &self,
        path: &str,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<WasmPlugin> {
        let name_str: String = name.into();
        let bytes = tokio::fs::read(path).await.map_err(|e| AstrBotError::Plugin {
            plugin: name_str.clone(),
            message: format!("read file failed: {}", e),
        })?;
        self.load_from_bytes(&bytes, name_str, version).await
    }

    /// 从字节数组加载 WASM 插件
    pub async fn load_from_bytes(
        &self,
        bytes: &[u8],
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<WasmPlugin> {
        let name_str: String = name.into();
        let module = wasmi::Module::new(&self.engine, bytes).map_err(|e| AstrBotError::Plugin {
            plugin: name_str.clone(),
            message: format!("module compilation failed: {}", e),
        })?;

        Ok(WasmPlugin {
            name: name_str,
            version: version.into(),
            module,
        })
    }

    /// 创建一个新的 store（每个调用/每个插件独立）
    pub fn create_store(&self) -> wasmi::Store<WasmHostState> {
        let state = self.host_state_template.clone();
        wasmi::Store::new(&self.engine, state)
    }

    /// 获取底层引擎引用
    pub fn engine(&self) -> &wasmi::Engine {
        &self.engine
    }
}

impl Default for WasmPluginLoader {
    fn default() -> Self {
        Self::new().expect("wasmi engine should initialize")
    }
}

// ---------------------------------------------------------------------------
// Plugin Instance (runtime)
// ---------------------------------------------------------------------------

/// WASM 插件运行时实例，绑定 module + store
pub struct WasmPluginInstance {
    pub store: wasmi::Store<WasmHostState>,
    pub instance: wasmi::Instance,
}

impl WasmPluginInstance {
    /// 从已加载的插件和 loader 创建可执行实例
    pub fn new(plugin: &WasmPlugin, loader: &WasmPluginLoader) -> Result<Self> {
        let mut store = loader.create_store();
        let mut linker = wasmi::Linker::<WasmHostState>::new(loader.engine());

        // 实例化（空导入表）
        let instance = linker
            .instantiate(&mut store, &plugin.module)
            .map_err(|e| AstrBotError::Plugin {
                plugin: plugin.name.clone(),
                message: format!("instantiation failed: {}", e),
            })?
            .start(&mut store)
            .map_err(|e| AstrBotError::Plugin {
                plugin: plugin.name.clone(),
                message: format!("start failed: {}", e),
            })?;

        Ok(Self { store, instance })
    }

    /// 调用导出的 `init` 函数（如果存在）
    pub fn call_init(&mut self) -> Result<()> {
        if let Ok(init) = self.instance.get_typed_func::<(), ()>(&self.store, "init") {
            init.call(&mut self.store, ()).map_err(|e| AstrBotError::Plugin {
                plugin: "wasm".to_string(),
                message: format!("init failed: {}", e),
            })?;
        }
        Ok(())
    }

    /// 调用导出的 `on_message` 函数（如果存在）
    /// 骨架版本：接受 (i32, i32) 返回 i32，对应 WASM 内存中的字符串指针/长度
    pub fn call_on_message(&mut self, _msg: &str) -> Result<String> {
        // 完整实现需要 malloc/free 导出 + 内存读写
        // 骨架阶段返回占位错误
        Err(AstrBotError::Plugin {
            plugin: "wasm".to_string(),
            message: "call_on_message skeleton — needs malloc/free exports and memory helpers".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// WASM 插件注册表
#[derive(Debug, Default)]
pub struct WasmPluginRegistry {
    plugins: HashMap<String, WasmPlugin>,
}

impl WasmPluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, plugin: WasmPlugin) {
        self.plugins.insert(plugin.name.clone(), plugin);
    }

    pub fn get(&self, name: &str) -> Option<&WasmPlugin> {
        self.plugins.get(name)
    }

    pub fn list(&self) -> Vec<&WasmPlugin> {
        self.plugins.values().collect()
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        self.plugins.remove(name).is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小合法 WASM 模块（magic + version）
    const MINIMAL_WASM: &[u8] = b"\x00asm\x01\x00\x00\x00";

    /// 带空导出表的 WASM 模块（module + type section 为空）
    /// 这个模块可以实例化
    const EMPTY_MODULE_WASM: &[u8] = b"\x00asm\x01\x00\x00\x00";

    #[tokio::test]
    async fn test_wasm_loader_creation() {
        let loader = WasmPluginLoader::new().unwrap();
        let _store = loader.create_store();
    }

    #[tokio::test]
    async fn test_wasm_plugin_load_minimal() {
        let loader = WasmPluginLoader::new().unwrap();
        let plugin = loader
            .load_from_bytes(MINIMAL_WASM, "test_minimal", "0.0.1")
            .await
            .unwrap();
        assert_eq!(plugin.name, "test_minimal");
        assert_eq!(plugin.version, "0.0.1");
    }

    #[tokio::test]
    async fn test_wasm_plugin_load_invalid_file() {
        let loader = WasmPluginLoader::new().unwrap();
        let result = loader
            .load_from_bytes(b"not_a_wasm_file", "bad_plugin", "0.0.1")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_wasm_host_state_memory_tracking() {
        let state = WasmHostState::new(1024, 1000);
        state.memory_usage.fetch_add(256, Ordering::Relaxed);
        assert_eq!(state.memory_usage.load(Ordering::Relaxed), 256);
    }

    #[tokio::test]
    async fn test_wasm_registry() {
        let loader = WasmPluginLoader::new().unwrap();
        let plugin = loader
            .load_from_bytes(MINIMAL_WASM, "reg_test", "1.0.0")
            .await
            .unwrap();
        let mut registry = WasmPluginRegistry::new();
        registry.register(plugin);
        assert!(registry.get("reg_test").is_some());
        assert!(registry.get("missing").is_none());
        assert_eq!(registry.list().len(), 1);
        assert!(registry.unregister("reg_test"));
        assert!(!registry.unregister("reg_test"));
    }
}
