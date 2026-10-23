use std::time::Duration;

use raindrop_ai_content::{AiContentService, HostReady, OfficialAiPlugin};
use rutis::{Ctx, FiberState};

#[tokio::test]
async fn official_service_waits_for_host_and_cancels_on_dependency_removal() {
    let ctx = Ctx::root().expect("Rutis root");
    let official = ctx.plugin(OfficialAiPlugin::default());
    (&official)
        .await
        .expect("dependency waiting is not a failure");
    assert_ne!(official.state().state, FiberState::Active);
    assert!(ctx.get::<AiContentService>().is_none());

    let host = ctx.provide(HostReady).expect("host dependency");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if official.state().state == FiberState::Active {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("official service should activate");
    let service = ctx
        .get::<AiContentService>()
        .expect("typed official service");
    assert!(!service.is_cancelled());

    host.dispose().await.expect("dependency removal");
    tokio::time::timeout(Duration::from_secs(2), service.cancelled())
        .await
        .expect("dependency removal cancels the old service");
    ctx.shutdown().await.expect("runtime cleanup");
    assert!(ctx.get::<AiContentService>().is_none());
}

#[tokio::test]
async fn shutdown_removes_the_official_service_and_cancels_outstanding_handles() {
    let ctx = Ctx::root().expect("Rutis root");
    ctx.provide(HostReady).expect("host dependency");
    let official = ctx.plugin(OfficialAiPlugin::default());
    (&official).await.expect("official startup");
    let service = ctx
        .get::<AiContentService>()
        .expect("typed official service");
    ctx.shutdown().await.expect("runtime cleanup");
    assert!(service.is_cancelled());
    assert!(ctx.get::<AiContentService>().is_none());
}
