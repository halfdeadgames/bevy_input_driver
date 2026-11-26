//! Tracks whether the render pipelines the game uses have finished compiling.

use std::{
    collections::BTreeMap,
    iter,
    path::Path,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    },
};

use bevy::{
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        render_resource::{CachedPipeline, CachedPipelineState, PipelineCache, PipelineDescriptor},
    },
    shader::{Shader, ShaderCacheError},
};
use serde::Serialize;

/// Render frames in a row with nothing compiling and no new pipelines before the game counts as
/// drawn, unless a `wait_until_drawn` command asks for another number with `settle_frames`.
/// Pipelines only queue once their assets load, so one quiet frame is not enough by default.
pub const SETTLED_RENDER_FRAMES: u32 = 3;

/// A render pipeline whose shaders failed to compile and will not be retried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailedPipeline {
    /// The pipeline descriptor's label, if it has one.
    pub label: Option<String>,
    /// Asset paths of the pipeline's shaders, without duplicates; shaders without a path are
    /// left out.
    pub shader_paths: Vec<String>,
    /// Bevy's full error text for the failure.
    pub error: String,
}

#[derive(Debug, Default)]
struct PipelineCounts {
    failed_details: Mutex<Vec<FailedPipeline>>,
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

    /// The label, shader paths, and error text of each pipeline that failed for good, as of the
    /// latest render frame.
    #[must_use]
    pub fn failed_pipeline_details(&self) -> Vec<FailedPipeline> {
        self.0
            .failed_details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // Compares before cloning, so a frame with an unchanged list copies nothing.
    pub(crate) fn copy_failed_pipelines_into(&self, destination: &mut Vec<FailedPipeline>) {
        let details = self
            .0
            .failed_details
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *destination != *details {
            destination.clone_from(&details);
        }
    }

    fn publish_failed_details(&self, details: Vec<FailedPipeline>) {
        *self
            .0
            .failed_details
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = details;
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
        self.quiet_render_frames() >= SETTLED_RENDER_FRAMES
    }

    /// Render frames in a row, up to the latest one, that had at least one pipeline, none
    /// pending, and no new ones. Saturates at `u32::MAX`.
    #[must_use]
    pub fn quiet_render_frames(&self) -> u32 {
        self.0.settled_frames.load(COUNTER_ORDERING)
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
    mut recorded_failures: Local<BTreeMap<usize, RecordedFailure>>,
) {
    let pending = pipeline_cache.waiting_pipelines().count();
    let mut total = 0;
    let mut failed = 0;
    let mut permanently_failed_indices = Vec::new();
    let mut failures_changed = false;
    for (index, pipeline) in pipeline_cache.pipelines().enumerate() {
        total += 1;
        if matches!(pipeline.state, CachedPipelineState::Err(_)) {
            failed += 1;
        }
        if permanent_error(&pipeline.state).is_some() {
            permanently_failed_indices.push(index);
            failures_changed |= record_failure_if_changed(&mut recorded_failures, index, pipeline);
        }
    }
    let recorded_before = recorded_failures.len();
    recorded_failures.retain(|index, _| permanently_failed_indices.contains(index));
    failures_changed |= recorded_failures.len() != recorded_before;
    if failures_changed {
        readiness.publish_failed_details(
            recorded_failures
                .values()
                .map(|recorded| recorded.description.clone())
                .collect(),
        );
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

struct RecordedFailure {
    error_detail: String,
    description: FailedPipeline,
}

// A hot-reloaded shader can fail again at the same index with new text, so the error detail is
// compared, not just the index. The detail is a borrowed string, so the compare allocates nothing.
fn record_failure_if_changed(
    recorded_failures: &mut BTreeMap<usize, RecordedFailure>,
    index: usize,
    pipeline: &CachedPipeline,
) -> bool {
    let Some(error_detail) = permanent_error(&pipeline.state).map(permanent_error_detail) else {
        return false;
    };
    if recorded_failures
        .get(&index)
        .is_some_and(|recorded| recorded.error_detail == error_detail)
    {
        return false;
    }
    let Some(description) = describe_failed_pipeline(pipeline) else {
        return false;
    };
    recorded_failures.insert(
        index,
        RecordedFailure {
            error_detail: error_detail.to_owned(),
            description,
        },
    );
    true
}

fn permanent_error_detail(error: &ShaderCacheError) -> &str {
    match error {
        ShaderCacheError::ProcessShaderError(detail)
        | ShaderCacheError::CreateShaderModule(detail) => detail,
        ShaderCacheError::ShaderNotLoaded(_) | ShaderCacheError::ShaderImportNotYetAvailable => "",
    }
}

// Bevy retries a pipeline that failed on a missing shader or import, so only the other errors are
// final.
fn permanent_error(state: &CachedPipelineState) -> Option<&ShaderCacheError> {
    match state {
        CachedPipelineState::Err(
            error @ (ShaderCacheError::ProcessShaderError(_)
            | ShaderCacheError::CreateShaderModule(_)),
        ) => Some(error),
        _ => None,
    }
}

fn describe_failed_pipeline(pipeline: &CachedPipeline) -> Option<FailedPipeline> {
    let error = permanent_error(&pipeline.state)?;
    Some(FailedPipeline {
        label: pipeline_label(&pipeline.descriptor),
        shader_paths: shader_asset_paths(&pipeline.descriptor),
        error: error.to_string(),
    })
}

fn pipeline_label(descriptor: &PipelineDescriptor) -> Option<String> {
    let label = match descriptor {
        PipelineDescriptor::RenderPipelineDescriptor(render) => &render.label,
        PipelineDescriptor::ComputePipelineDescriptor(compute) => &compute.label,
        PipelineDescriptor::MeshPipelineDescriptor(mesh) => &mesh.label,
    };
    label.as_ref().map(ToString::to_string)
}

fn shader_asset_paths(descriptor: &PipelineDescriptor) -> Vec<String> {
    let handles: Vec<&Handle<Shader>> = match descriptor {
        PipelineDescriptor::RenderPipelineDescriptor(render) => iter::once(&render.vertex.shader)
            .chain(render.fragment.iter().map(|fragment| &fragment.shader))
            .collect(),
        PipelineDescriptor::ComputePipelineDescriptor(compute) => vec![&compute.shader],
        PipelineDescriptor::MeshPipelineDescriptor(mesh) => mesh
            .task
            .iter()
            .map(|task| &task.shader)
            .chain(iter::once(&mesh.mesh.shader))
            .chain(mesh.fragment.iter().map(|fragment| &fragment.shader))
            .collect(),
    };
    let mut paths: Vec<String> = Vec::new();
    for path in handles
        .into_iter()
        .filter_map(Handle::path)
        .map(|path| forward_slash_path(path.path()))
    {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

// Asset paths are written with `/` in Bevy, but embedded ones are joined with the OS separator,
// so a script would otherwise see different paths on Windows.
fn forward_slash_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests {
    use bevy::render::render_resource::RenderPipelineDescriptor;

    use super::*;

    fn failed_with(error: ShaderCacheError) -> CachedPipeline {
        CachedPipeline {
            descriptor: PipelineDescriptor::RenderPipelineDescriptor(Box::new(
                RenderPipelineDescriptor {
                    label: Some("test".into()),
                    ..default()
                },
            )),
            state: CachedPipelineState::Err(error),
        }
    }

    // A handle with an asset path needs a running asset server, so paths are not covered here.
    #[test]
    fn describes_permanent_failures_with_label_and_full_error() {
        let long_message = "x".repeat(5000);
        let process = describe_failed_pipeline(&failed_with(ShaderCacheError::ProcessShaderError(
            long_message.clone(),
        )))
        .expect("process errors are permanent");
        assert_eq!(process.label.as_deref(), Some("test"));
        assert!(process.shader_paths.is_empty());
        assert!(process.error.ends_with(&long_message));
        let module = describe_failed_pipeline(&failed_with(ShaderCacheError::CreateShaderModule(
            "bad".to_owned(),
        )))
        .expect("module errors are permanent");
        assert_eq!(module.error, "Could not create shader module: bad");
    }

    #[test]
    fn a_new_error_at_the_same_index_replaces_the_old_text() {
        let mut recorded = BTreeMap::new();
        let first = failed_with(ShaderCacheError::ProcessShaderError("first".to_owned()));
        assert!(record_failure_if_changed(&mut recorded, 0, &first));
        assert!(!record_failure_if_changed(&mut recorded, 0, &first));
        let second = failed_with(ShaderCacheError::ProcessShaderError("second".to_owned()));
        assert!(record_failure_if_changed(&mut recorded, 0, &second));
        assert!(recorded[&0].description.error.ends_with("second"));
    }

    #[test]
    fn shader_paths_use_forward_slashes_on_every_platform() {
        let nested: std::path::PathBuf = ["bevy_sprite_render", "mesh2d", "mesh2d.wesl"]
            .iter()
            .collect();
        assert_eq!(
            forward_slash_path(&nested),
            "bevy_sprite_render/mesh2d/mesh2d.wesl"
        );
    }

    #[test]
    fn retried_errors_and_other_states_are_not_described() {
        assert!(
            describe_failed_pipeline(&failed_with(ShaderCacheError::ShaderImportNotYetAvailable))
                .is_none()
        );
        assert!(
            describe_failed_pipeline(&failed_with(ShaderCacheError::ShaderNotLoaded(
                AssetId::default()
            )))
            .is_none()
        );
        let mut queued = failed_with(ShaderCacheError::ShaderImportNotYetAvailable);
        queued.state = CachedPipelineState::Queued;
        assert!(describe_failed_pipeline(&queued).is_none());
    }

    #[test]
    fn status_follows_the_shared_failures_and_clears_without_a_renderer() {
        let readiness = RenderReadiness::default();
        let failure = FailedPipeline {
            label: None,
            shader_paths: Vec::new(),
            error: "boom".to_owned(),
        };
        readiness.publish_failed_details(vec![failure.clone()]);
        let mut status = crate::status::DriverStatus::default();
        status.refresh_render_readiness(Some(&readiness));
        assert_eq!(status.failed_pipelines, vec![failure.clone()]);
        readiness.publish_failed_details(Vec::new());
        status.refresh_render_readiness(Some(&readiness));
        assert!(status.failed_pipelines.is_empty());
        readiness.publish_failed_details(vec![failure]);
        status.refresh_render_readiness(None);
        assert!(status.failed_pipelines.is_empty());
    }

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
        assert_eq!(readiness.quiet_render_frames(), SETTLED_RENDER_FRAMES);
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
    fn status_reports_render_frames_or_none_without_a_renderer() {
        let readiness = RenderReadiness::default();
        readiness.publish(snapshot(1, 0), 0);
        readiness.publish(snapshot(1, 0), 1);
        let mut status = crate::status::DriverStatus::default();
        status.refresh_render_readiness(Some(&readiness));
        assert_eq!(status.render_frame_count, Some(2));
        status.refresh_render_readiness(None);
        assert_eq!(status.render_frame_count, None);
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
