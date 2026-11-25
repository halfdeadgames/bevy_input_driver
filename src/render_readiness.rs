//! Tracks whether the render pipelines the game uses have finished compiling.

use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicUsize, Ordering},
};

use bevy::{
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        render_resource::{CachedPipelineState, PipelineCache},
    },
};

/// Render frames in a row with nothing compiling and no new pipelines before the game counts as
/// drawn. Pipelines only queue once their assets load, so one quiet frame is not enough.
pub const SETTLED_RENDER_FRAMES: u32 = 3;

#[derive(Debug, Default)]
struct PipelineCounts {
    pending: AtomicUsize,
    failed: AtomicUsize,
    settled_frames: AtomicU32,
    total: AtomicUsize,
    frames_published: AtomicU32,
}

/// Pipeline compilation progress, written by the render world and read by the main world.
///
/// The same shared counters are inserted into both worlds. It is present in the main world only
/// when the app has a render sub-app.
#[derive(Resource, Debug, Clone, Default)]
pub struct RenderReadiness(Arc<PipelineCounts>);

// The frame counter is written last with `Release` and read first with `Acquire`, so a reader
// that sees a frame also sees that frame's other counters. The rest may be one frame stale.
const COUNTER_ORDERING: Ordering = Ordering::Relaxed;

impl RenderReadiness {
    /// Pipelines still queued or compiling after the latest render frame.
    #[must_use]
    pub fn pending_pipelines(&self) -> usize {
        self.0.pending.load(COUNTER_ORDERING)
    }

    /// Pipelines that failed to compile and will not be retried, as of the latest settled frame.
    #[must_use]
    pub fn failed_pipelines(&self) -> usize {
        self.0.failed.load(COUNTER_ORDERING)
    }

    /// Pipelines in the cache after the latest render frame, compiled or not.
    #[must_use]
    pub fn total_pipelines(&self) -> usize {
        self.0.total.load(COUNTER_ORDERING)
    }

    /// Render frames finished so far, wrapping at `u32::MAX`. Read it before the other counters:
    /// they are then at least as new as the frame it reports.
    #[must_use]
    pub fn render_frames_published(&self) -> u32 {
        self.0.frames_published.load(Ordering::Acquire)
    }

    /// True once [`SETTLED_RENDER_FRAMES`] render frames in a row had at least one pipeline,
    /// none pending, and no new ones.
    #[must_use]
    pub fn is_drawn(&self) -> bool {
        self.0.settled_frames.load(COUNTER_ORDERING) >= SETTLED_RENDER_FRAMES
    }

    fn publish(&self, snapshot: PipelineSnapshot, previous_total: usize) {
        let counts = &self.0;
        counts.pending.store(snapshot.pending, COUNTER_ORDERING);
        counts.total.store(snapshot.total, COUNTER_ORDERING);
        if snapshot.is_settled_since(previous_total) {
            counts.failed.store(snapshot.failed, COUNTER_ORDERING);
            let settled = counts
                .settled_frames
                .load(COUNTER_ORDERING)
                .saturating_add(1);
            counts.settled_frames.store(settled, COUNTER_ORDERING);
        } else {
            counts.settled_frames.store(0, COUNTER_ORDERING);
        }
        counts.frames_published.fetch_add(1, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PipelineSnapshot {
    total: usize,
    pending: usize,
    failed: usize,
}

impl PipelineSnapshot {
    fn is_settled_since(self, previous_total: usize) -> bool {
        self.total > 0 && self.pending == 0 && self.total == previous_total
    }
}

/// Shares one [`RenderReadiness`] between the main world and the render world.
///
/// Without a render sub-app it does nothing.
pub struct DriverRenderReadinessPlugin;

impl Plugin for DriverRenderReadinessPlugin {
    fn build(&self, _app: &mut App) {}

    // `RenderPlugin` creates the render sub-app in its own `build`, so looking for it in `finish`,
    // after every plugin is built, works whichever order the game added the plugins in.
    fn finish(&self, app: &mut App) {
        let readiness = RenderReadiness::default();
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.insert_resource(readiness.clone()).add_systems(
            Render,
            publish_pipeline_readiness.in_set(RenderSystems::Cleanup),
        );
        app.insert_resource(readiness);
    }
}

// Runs after `PipelineCache::process_queue`, so pipelines queued this frame are already waiting,
// and a pipeline in `Err` that is not waiting has failed for good.
fn publish_pipeline_readiness(
    pipeline_cache: Res<PipelineCache>,
    readiness: Res<RenderReadiness>,
    mut previous_total: Local<usize>,
) {
    let pending = pipeline_cache.waiting_pipelines().count();
    let mut total = 0;
    let mut failed = 0;
    for pipeline in pipeline_cache.pipelines() {
        total += 1;
        if matches!(pipeline.state, CachedPipelineState::Err(_)) {
            failed += 1;
        }
    }
    readiness.publish(
        PipelineSnapshot {
            total,
            pending,
            failed,
        },
        *previous_total,
    );
    *previous_total = total;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(total: usize, pending: usize) -> PipelineSnapshot {
        PipelineSnapshot {
            total,
            pending,
            failed: 0,
        }
    }

    #[test]
    fn drawn_after_enough_quiet_frames_with_a_steady_count() {
        let readiness = RenderReadiness::default();
        readiness.publish(snapshot(4, 0), 0);
        for _ in 0..SETTLED_RENDER_FRAMES - 1 {
            readiness.publish(snapshot(4, 0), 4);
        }
        assert!(!readiness.is_drawn());
        readiness.publish(snapshot(4, 0), 4);
        assert!(readiness.is_drawn());
    }

    #[test]
    fn pending_or_new_pipelines_restart_the_count() {
        let readiness = RenderReadiness::default();
        for _ in 0..SETTLED_RENDER_FRAMES {
            readiness.publish(snapshot(2, 0), 2);
        }
        assert!(readiness.is_drawn());
        readiness.publish(snapshot(3, 0), 2);
        assert!(!readiness.is_drawn());
        readiness.publish(snapshot(3, 1), 3);
        assert_eq!(readiness.pending_pipelines(), 1);
        assert!(!readiness.is_drawn());
    }

    #[test]
    fn every_publish_counts_a_render_frame_and_the_total() {
        let readiness = RenderReadiness::default();
        readiness.publish(snapshot(5, 2), 0);
        readiness.publish(snapshot(6, 0), 5);
        assert_eq!(readiness.render_frames_published(), 2);
        assert_eq!(readiness.total_pipelines(), 6);
    }

    #[test]
    fn finds_a_render_sub_app_added_after_the_plugin() {
        let mut app = App::new();
        app.add_plugins(DriverRenderReadinessPlugin);
        app.insert_sub_app(RenderApp, SubApp::new());
        app.finish();
        assert!(app.world().contains_resource::<RenderReadiness>());
        let render_world = app
            .get_sub_app(RenderApp)
            .expect("render sub-app was inserted")
            .world();
        assert!(render_world.contains_resource::<RenderReadiness>());
    }

    #[test]
    fn without_a_render_sub_app_nothing_is_inserted() {
        let mut app = App::new();
        app.add_plugins(DriverRenderReadinessPlugin);
        app.finish();
        assert!(!app.world().contains_resource::<RenderReadiness>());
    }

    #[test]
    fn no_pipelines_is_never_drawn() {
        let readiness = RenderReadiness::default();
        for _ in 0..10 {
            readiness.publish(snapshot(0, 0), 0);
        }
        assert!(!readiness.is_drawn());
    }
}
