use std::sync::Arc;

use raindrop_ai_content::{AiContentService, HostReady, OfficialAiPlugin};
use rutis::{Ctx, FiberState, FiberView};

use super::{PluginRuntimeError, PluginRuntimeErrorKind};

#[derive(Clone)]
pub struct PluginRuntime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    ctx: Ctx,
    official: FiberView,
}

impl PluginRuntime {
    pub fn new() -> Result<Self, PluginRuntimeError> {
        let ctx = Ctx::root().map_err(|_| unavailable())?;
        ctx.provide(HostReady).map_err(|_| unavailable())?;
        let official = ctx.plugin(OfficialAiPlugin::default());
        Ok(Self {
            inner: Arc::new(RuntimeInner { ctx, official }),
        })
    }

    pub async fn ready(&self) -> Result<(), PluginRuntimeError> {
        self.service().await.map(|_| ())
    }

    pub async fn shutdown(&self) -> Result<(), PluginRuntimeError> {
        self.inner.ctx.shutdown().await.map_err(|_| unavailable())
    }

    pub(crate) async fn service(&self) -> Result<Arc<AiContentService>, PluginRuntimeError> {
        (&self.inner.official).await.map_err(|_| unavailable())?;
        if self.inner.official.state().state != FiberState::Active {
            return Err(unavailable());
        }
        self.inner
            .ctx
            .get::<AiContentService>()
            .filter(|service| !service.is_cancelled())
            .ok_or_else(unavailable)
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        drop(self.ctx.shutdown());
    }
}

fn unavailable() -> PluginRuntimeError {
    PluginRuntimeError::new(PluginRuntimeErrorKind::RuntimeUnavailable)
}
