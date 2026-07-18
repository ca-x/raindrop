use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use tokio::runtime::Handle;
use wasmtime::{Config, Engine};

use super::{PluginRuntimeError, PluginRuntimeErrorKind};

const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(10);
const MAX_WASM_STACK_BYTES: usize = 512 * 1024;

#[derive(Clone)]
pub struct PluginRuntime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    engine: Engine,
    epoch_ticker_stop: Arc<AtomicBool>,
    epoch_ticker: Option<JoinHandle<()>>,
}

impl PluginRuntime {
    pub fn new() -> Result<Self, PluginRuntimeError> {
        Handle::try_current()
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::RuntimeUnavailable))?;
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .wasm_component_model_async(true)
            .consume_fuel(true)
            .epoch_interruption(true)
            .max_wasm_stack(MAX_WASM_STACK_BYTES);
        let engine = Engine::new(&config)
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::RuntimeUnavailable))?;
        let epoch_ticker_stop = Arc::new(AtomicBool::new(false));
        let ticker_stop = Arc::clone(&epoch_ticker_stop);
        let ticker_engine = engine.clone();
        let epoch_ticker = thread::Builder::new()
            .name("raindrop-plugin-epoch".to_owned())
            .spawn(move || {
                while !ticker_stop.load(Ordering::Acquire) {
                    thread::park_timeout(EPOCH_TICK_INTERVAL);
                    if !ticker_stop.load(Ordering::Acquire) {
                        ticker_engine.increment_epoch();
                    }
                }
            })
            .map_err(|_| PluginRuntimeError::new(PluginRuntimeErrorKind::RuntimeUnavailable))?;

        Ok(Self {
            inner: Arc::new(RuntimeInner {
                engine,
                epoch_ticker_stop,
                epoch_ticker: Some(epoch_ticker),
            }),
        })
    }

    pub(crate) fn engine(&self) -> &Engine {
        &self.inner.engine
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        self.epoch_ticker_stop.store(true, Ordering::Release);
        if let Some(epoch_ticker) = self.epoch_ticker.take() {
            epoch_ticker.thread().unpark();
            let _ = epoch_ticker.join();
        }
    }
}
