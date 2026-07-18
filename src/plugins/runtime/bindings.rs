mod generated {
    wasmtime::component::bindgen!({
        path: "contracts/wit/raindrop-content-plugin-v1",
        world: "content-plugin-v1",
        imports: { default: async | trappable },
        exports: { default: async },
    });
}

pub use generated::ContentPluginV1;
pub use generated::exports::raindrop::content_plugin::content_plugin;
pub use generated::raindrop::content_plugin::{host_ai, host_mcp, types};
