//! Frame cost (config clouds.frame_timing): one GPU span per frame, from a timestamp before the first Core3d pass to one after the last
//! post-process pass, plus the wall time between frames. Prints `FRAMEMS gpu <mean> med <median> p10 <p10> max <max> | wall <mean> | n <frames read>` every
//! 60 frames (gpu mean, median, 10th percentile, max).
//! Why a frame span: on Metal (Apple Silicon) every pass's begin timestamp fires near the start of the frame and independent passes
//! overlap, so a per-pass span (PASSMS, FOGMS) is "frame start to this pass's end", not the pass's own cost. A feature's cost is the
//! FRAMEMS difference between a run with it and one without (tests/cost.rs).
//! A slot is resolved one frame after its end timestamp (at the next frame's start): on Metal an end-of-pass timestamp resolved in the same
//! command buffer still reads 0.
//! The readback never blocks: SLOTS query pairs, each mapped asynchronously and read when ready; a frame takes any free slot and is
//! skipped when none is (n counts the frames actually read).
//! Bounds: the span starts before Core3d's Prepass set and ends after its PostProcess set; GPU work outside them (Bevy's GPU preprocess,
//! the UI pass, upscaling to the output) is not in it. Nephthene's passes all are.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Instant;
use bevy::{
    core_pipeline::{schedule::Core3d, Core3dSystems},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::ViewTarget,
    },
};

/// Readback slots: headless, the CPU runs many frames ahead of the GPU and a slot's map completes 16+ frames after its write (4 slots
/// read 4-13 frames per window, 16 slots about 26). 256 bytes each. ponytail: raise it if n drops well below 60.
const SLOTS: usize = 64;
/// Byte stride of a slot in the resolve buffer: wgpu's QUERY_RESOLVE_BUFFER_ALIGNMENT.
const STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
/// Frames per printed mean, as PASSMS and FOGMS.
const WINDOW: u32 = 60;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Slot { Free, Ended, Written, Mapping }

#[derive(Resource)]
struct FrameTiming {
    qs: wgpu::QuerySet, resolve: wgpu::Buffer, read: Vec<wgpu::Buffer>, ready: Vec<Arc<AtomicBool>>, state: [Slot; SLOTS],
    /// The slot this frame writes, None when it is still in flight (skip) or the frame's span is already written (a second view).
    cur: Option<usize>,
    gpu: Vec<f64>, wall: Vec<f64>, last: Option<Instant>, frames: u32,
}

#[derive(Resource)]
struct FrameTimingOn(bool);

pub fn plugin(app: &mut App, on: bool) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app.insert_resource(FrameTimingOn(on))
        .add_systems(RenderStartup, init)
        .add_systems(Render, (begin_frame.in_set(RenderSystems::PrepareResources), read.in_set(RenderSystems::Cleanup)))
        .add_systems(Core3d, (frame_start.before(Core3dSystems::Prepass), frame_end.after(Core3dSystems::PostProcess)));
}

fn init(mut commands: Commands, device: Res<RenderDevice>, on: Res<FrameTimingOn>) {
    if !on.0 || !device.features().contains(WgpuFeatures::TIMESTAMP_QUERY) { return; }
    let d = device.wgpu_device();
    let buf = |usage, label| d.create_buffer(&BufferDescriptor { label: Some(label), size: 16, usage, mapped_at_creation: false });
    commands.insert_resource(FrameTiming {
        qs: d.create_query_set(&wgpu::QuerySetDescriptor { label: Some("frame_ts"), ty: wgpu::QueryType::Timestamp, count: 2 * SLOTS as u32 }),
        resolve: d.create_buffer(&BufferDescriptor { label: Some("frame_ts_resolve"), size: STRIDE * SLOTS as u64, usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC, mapped_at_creation: false }),
        read: (0..SLOTS).map(|_| buf(BufferUsages::COPY_DST | BufferUsages::MAP_READ, "frame_ts_read")).collect(),
        ready: (0..SLOTS).map(|_| default()).collect(), state: [Slot::Free; SLOTS], cur: None,
        gpu: Vec::new(), wall: Vec::new(), last: None, frames: 0,
    });
}

/// Pick this frame's slot: any free one, or none when all are in flight (the frame is skipped).
fn begin_frame(timing: Option<ResMut<FrameTiming>>) {
    let Some(mut t) = timing else { return };
    t.cur = t.state.iter().position(|s| *s == Slot::Free);
}

/// Resolve the slots last frame ended, then an empty pass on the view target writes this frame's first timestamp at its beginning.
fn frame_start(view: ViewQuery<&ViewTarget>, timing: Option<ResMut<FrameTiming>>, mut ctx: RenderContext) {
    let Some(mut t) = timing else { return };
    for s in 0..SLOTS {
        if t.state[s] != Slot::Ended { continue; }
        let (e, off, q) = (ctx.command_encoder(), STRIDE * s as u64, 2 * s as u32);
        e.resolve_query_set(&t.qs, q..q + 2, &t.resolve, off);
        e.copy_buffer_to_buffer(&t.resolve, off, &t.read[s], 0, 16);
        t.state[s] = Slot::Written;
    }
    let Some(slot) = t.cur else { return };
    drop(ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("frame_ts_start"), color_attachments: &[Some(view.into_inner().get_color_attachment())], depth_stencil_attachment: None,
        timestamp_writes: Some(wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: Some(2 * slot as u32), end_of_pass_write_index: None }),
        occlusion_query_set: None, multiview_mask: None,
    }));
}

/// An empty pass on the view target writes the frame's last timestamp at its end (it loads the target, so it follows every pass that
/// wrote it). The pair is resolved next frame (frame_start).
fn frame_end(view: ViewQuery<&ViewTarget>, timing: Option<ResMut<FrameTiming>>, mut ctx: RenderContext) {
    let Some(mut t) = timing else { return };
    let Some(slot) = t.cur.take() else { return };
    drop(ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("frame_ts_end"), color_attachments: &[Some(view.into_inner().get_color_attachment())], depth_stencil_attachment: None,
        timestamp_writes: Some(wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(2 * slot as u32 + 1) }),
        occlusion_query_set: None, multiview_mask: None,
    }));
    t.state[slot] = Slot::Ended;
}

/// After submit: start mapping the slots written this frame, read the ones that finished, record the wall time, print every WINDOW frames.
fn read(timing: Option<ResMut<FrameTiming>>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let Some(mut t) = timing else { return };
    let t = &mut *t;
    let now = Instant::now();
    if let Some(last) = t.last.replace(now) { t.wall.push((now - last).as_secs_f64() * 1e3); }
    for s in 0..SLOTS {
        if t.state[s] != Slot::Written { continue; }
        let flag = t.ready[s].clone();
        flag.store(false, Ordering::Release);
        t.read[s].slice(..).map_async(MapMode::Read, move |r| if r.is_ok() { flag.store(true, Ordering::Release) });
        t.state[s] = Slot::Mapping;
    }
    let _ = device.poll(PollType::Poll);
    let period = queue.get_timestamp_period() as f64 * 1e-6;
    for s in 0..SLOTS {
        if t.state[s] != Slot::Mapping || !t.ready[s].load(Ordering::Acquire) { continue; }
        let ts: Vec<u64> = t.read[s].slice(..).get_mapped_range().expect("frame timestamps").chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        t.read[s].unmap();
        t.state[s] = Slot::Free;
        if ts[0] > 0 && ts[1] >= ts[0] { t.gpu.push((ts[1] - ts[0]) as f64 * period); }
    }
    t.frames += 1;
    if t.frames == WINDOW {
        let mean = |v: &[f64]| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
        let mut g = t.gpu.clone();
        g.sort_by(f64::total_cmp);
        let pct = |p: f64| g.get(((g.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0.0);
        println!("FRAMEMS gpu {:.3} med {:.3} p10 {:.3} max {:.3} | wall {:.3} | n {}", mean(&g), pct(0.5), pct(0.1), pct(1.0), mean(&t.wall), g.len());
        (t.frames, t.gpu, t.wall) = (0, Vec::new(), Vec::new());
    }
}
