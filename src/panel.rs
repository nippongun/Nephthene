//! Weather panel: a mouse-driven panel (bevy_feathers widgets) on the left. Thin Bevy adapter; the state and maths are in
//! `nephthene_core::dash` (unit-tested). Open/close: ` (Backquote) or the [x]; `Panel.0.open` is the state, so a host can gate its own
//! input on it (`Res<Panel>`). All overrides are runtime-only; sizes are px, untuned.
use crate::{clouds::CloudParams, rain::RainState, sky::Weather};
use bevy::{
    asset::RenderAssetUsages, feathers::{FeathersPlugins, controls::*, dark_theme::create_dark_theme, display::{caption, label, label_dim}, theme::UiTheme},
    gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore}, image::ImageSampler, input::mouse::{MouseScrollUnit, MouseWheel}, pbr::AtmosphereSettings,
    picking::events::{PointerClick, PointerMove, PointerOut}, prelude::*, render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::{Checked, RelativeCursorPosition}, ui_widgets::{Activate, Checkbox, SetSelected, SliderPrecision, SliderValue, ValueChange},
};
use nephthene_core::{self as w, dash::{self as c, Channel, Ctl}};

const PANEL_W: f32 = 430.0;
const MAP_W: f32 = PANEL_W - 24.0;

#[derive(Resource)]
pub struct Panel(pub Ctl, pub String);
#[derive(Resource)]
struct MapImage(Handle<Image>);
/// Map uv (0..1, v down from the north edge) under the cursor.
#[derive(Resource, Default)]
struct MapHover(Option<Vec2>);
#[derive(Component)]
struct DashPanel;
#[derive(Component)]
struct MapView;
/// Which control an event came from.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub enum Wid { Close, Hour, PlayPause, Rate, CovMode, Cov, WindDir, WindSpeed, Rain, Reset, FogOn, FogMode, FogAmount, FogBurn, FogDepth, FogVis, FogWisp, StormsOn, StormNow, Clouds, Shadows, Debug, UpdateDiv, Channel, Giz(usize) }
/// Option index a select currently shows (so a state change elsewhere moves it).
#[derive(Component)]
struct SelShown(usize);
#[derive(Component, Clone, Copy, Default)]
enum Txt { #[default] Time, Play, Here, Air, Climate, Oro, Rain, Sun, Frame, Hover, Pin, Fog, Storm }
#[derive(Component, Clone, Copy, PartialEq)]
enum Mark { Cam, Heading, Pin, Storm(usize) }

pub struct PanelPlugin;
impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        let open = app.world().resource::<Weather>().cfg.sim.panel_open;
        app.add_plugins(FeathersPlugins).insert_resource(UiTheme(create_dark_theme()))
            .insert_resource(Panel(Ctl { open, ..default() }, String::new())).init_resource::<MapHover>()
            .add_observer(on_activate).add_observer(on_f32).add_observer(on_bool).add_observer(on_select).add_observer(on_map_move).add_observer(on_map_out).add_observer(on_map_click)
            .add_systems(PostStartup, (spawn, gizmo_style))
            .add_systems(Update, (input, apply.after(crate::NephtheneSet::Tick).before(crate::NephtheneSet::Drive), sync_widgets, refresh_ui, scroll, draw_gizmos).chain().after(crate::NephtheneSet::Tick));
    }
}

fn gizmo_style(mut store: ResMut<GizmoConfigStore>) {
    let (cfg, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    cfg.line.width = 3.0;
    cfg.depth_bias = -1.0; // in front of terrain: debug lines must not z-fight the ground
}

fn px(v: f32) -> Val { Val::Px(v) }
fn col(c: [f32; 3]) -> Color { Color::srgb(c[0], c[1], c[2]) }

// ---- spawn helpers: plain nodes are bundles, widgets are feathers scenes parented under them ----
fn row(cm: &mut Commands, parent: Entity) -> Entity {
    cm.spawn((Node { align_items: AlignItems::Center, column_gap: px(8.0), ..default() }, ChildOf(parent))).id()
}
const ORANGE: Color = Color::srgb(1.0, 0.55, 0.15);
const CREAM: Color = Color::srgb(0.96, 0.92, 0.82);
const PANEL_BG: Color = Color::srgba(0.07, 0.07, 0.09, 0.94);
const RING: Color = Color::srgb(0.35, 0.35, 0.4);
fn heading(cm: &mut Commands, parent: Entity, t: &str) {
    cm.spawn((Text::new(t), TextFont { font_size: FontSize::Px(20.0), ..default() },
        TextColor(ORANGE), Node { margin: UiRect::top(px(6.0)), ..default() }, ChildOf(parent)));
}
fn lbl(cm: &mut Commands, parent: Entity, t: &str) { cm.spawn_scene(label(t.to_string())).insert(ChildOf(parent)); }
fn dim(cm: &mut Commands, parent: Entity, t: &str) { cm.spawn_scene(label_dim(t.to_string())).insert(ChildOf(parent)); }
fn select(cm: &mut Commands, parent: Entity, wid: Wid, names: &[&str], cur: usize) {
    cm.spawn_scene(bsn! { @FeathersSelect { @options: list_rows_from_strings(names.iter().copied(), Some(cur)) } Node { flex_grow: 1.0 } })
        .insert((wid, SelShown(cur), ChildOf(parent)));
}
fn slider(cm: &mut Commands, parent: Entity, wid: Wid, min: f32, max: f32) {
    cm.spawn_scene(bsn! { @FeathersSlider { @min: min, @max: max } SliderPrecision(2) }).insert((wid, ChildOf(parent)));
}
fn number(cm: &mut Commands, parent: Entity, wid: Wid, lo: f32, hi: f32) {
    cm.spawn_scene(bsn! { @FeathersNumberInput NumberInputPrecision(0) HardLimit::f32(lo..=hi) Node { flex_grow: 1.0, max_width: Val::Px(120.0) } })
        .insert((wid, ChildOf(parent)));
}
fn toggle(cm: &mut Commands, parent: Entity, wid: Wid) { cm.spawn_scene(bsn! { @FeathersToggleSwitch }).insert((wid, ChildOf(parent))); }
fn button(cm: &mut Commands, parent: Entity, wid: Wid, text: &str, k: Option<Txt>) {
    let text = text.to_string();
    let b = cm.spawn_scene(bsn! { @FeathersButton { @caption: bsn! { @caption(text) } } }).insert((wid, ChildOf(parent))).id();
    if let Some(k) = k { cm.queue(move |w: &mut World| { // the caption text is the button's descendant
        let t = w.entity(b).get::<Children>().into_iter().flatten().copied().collect::<Vec<_>>();
        for e in t { if w.entity(e).contains::<Text>() { w.entity_mut(e).insert(k); } else if let Some(ch) = w.entity(e).get::<Children>() { let ch: Vec<_> = ch.iter().collect(); for g in ch { if w.entity(g).contains::<Text>() { w.entity_mut(g).insert(k); } } } }
    }); }
}
fn checkbox(cm: &mut Commands, parent: Entity, wid: Wid, text: &str) {
    let text = text.to_string();
    cm.spawn_scene(bsn! { @FeathersCheckbox { @caption: bsn! { @caption(text) } } }).insert((wid, ChildOf(parent)));
}

fn spawn(mut commands: Commands, wx: Res<Weather>, dash: Res<Panel>, mut images: ResMut<Assets<Image>>) {
    let map = &wx.map;
    let mut img = Image::new(Extent3d { width: map.width, height: map.height, depth_or_array_layers: 1 }, TextureDimension::D2,
        c::effective_rgba(map, &dash.0), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    img.sampler = ImageSampler::nearest();
    let handle = images.add(img);
    commands.insert_resource(MapImage(handle.clone()));
    let map_h = MAP_W * map.height as f32 / map.width as f32;
    let cm = &mut commands;
    let base = wx.cfg.clock.minutes_per_second / 60.0;
    let rate_i = c::rate_index(if base > 0.0 { wx.rate / base } else { 0.0 });
    let root = cm.spawn((DashPanel, RelativeCursorPosition::default(), BackgroundColor(PANEL_BG), BorderColor::all(RING),
        Node { position_type: PositionType::Absolute, top: px(64.0), left: px(20.0), bottom: px(20.0), width: px(PANEL_W), flex_direction: FlexDirection::Column, row_gap: px(6.0),
            padding: UiRect::all(px(12.0)), border: UiRect::all(px(3.0)), border_radius: BorderRadius::all(px(14.0)), overflow: Overflow::scroll_y(),
            display: if dash.0.open { Display::Flex } else { Display::None }, ..default() })).id();
    // header
    let h = row(cm, root);
    cm.spawn((Text::new("WEATHER"), TextFont { font_size: FontSize::Px(26.0), ..default() }, TextColor(CREAM), ChildOf(h)));
    cm.spawn((Node { flex_grow: 1.0, ..default() }, ChildOf(h)));
    cm.spawn_scene(label_dim("` to close".to_string())).insert(ChildOf(h));
    button(cm, h, Wid::Close, "x", None);
    // clock
    heading(cm, root, "Clock");
    let r = row(cm, root);
    cm.spawn_scene(label_dim("--:--".to_string())).insert((Txt::Time, ChildOf(r)));
    cm.spawn((Node { flex_grow: 1.0, ..default() }, ChildOf(r)));
    button(cm, r, Wid::PlayPause, "Pause", Some(Txt::Play));
    let r = row(cm, root);
    slider(cm, r, Wid::Hour, 0.0, 24.0);
    let r = row(cm, root);
    lbl(cm, r, "Rate");
    select(cm, r, Wid::Rate, &c::RATE_NAMES, rate_i);
    // weather
    heading(cm, root, "Weather");
    let r = row(cm, root);
    lbl(cm, r, "Coverage");
    select(cm, r, Wid::CovMode, &["map", "override"], dash.0.coverage.is_some() as usize);
    let r = row(cm, root);
    slider(cm, r, Wid::Cov, 0.0, 1.0);
    let r = row(cm, root);
    lbl(cm, r, "Wind from (deg)");
    number(cm, r, Wid::WindDir, 0.0, 360.0);
    lbl(cm, r, "speed (m/s)");
    number(cm, r, Wid::WindSpeed, 0.0, 30.0);
    let r = row(cm, root);
    lbl(cm, r, "Rain");
    toggle(cm, r, Wid::Rain);
    dim(cm, r, "(forces rain: streaks, splashes, wet ground)");
    cm.spawn((Node { flex_grow: 1.0, ..default() }, ChildOf(r)));
    button(cm, r, Wid::Reset, "Reset all", None);
    // fog: the switch, the amount (model or forced), and the knobs that shape the pool and its top
    heading(cm, root, "Fog");
    let r = row(cm, root);
    lbl(cm, r, "Ground fog");
    toggle(cm, r, Wid::FogOn);
    let r = row(cm, root);
    lbl(cm, r, "Amount");
    select(cm, r, Wid::FogMode, &["model", "force"], wx.cfg.ground_fog.force.is_some() as usize);
    let r = row(cm, root);
    slider(cm, r, Wid::FogAmount, 0.0, 1.0);
    let r = row(cm, root);
    lbl(cm, r, "Burn-off per hour");
    slider(cm, r, Wid::FogBurn, 0.0, 3.0);
    let r = row(cm, root);
    lbl(cm, r, "Pool depth (m)");
    slider(cm, r, Wid::FogDepth, 20.0, 400.0);
    let r = row(cm, root);
    lbl(cm, r, "Visibility (m)");
    slider(cm, r, Wid::FogVis, 30.0, 1000.0);
    let r = row(cm, root);
    lbl(cm, r, "Wisps (m)");
    slider(cm, r, Wid::FogWisp, 0.0, 120.0);
    cm.spawn_scene(label_dim(String::new())).insert((Txt::Fog, ChildOf(root)));
    // storms: the switch, a summon, and the readout (nearest cell, storm weather here)
    heading(cm, root, "Storms");
    let r = row(cm, root);
    lbl(cm, r, "Storms");
    toggle(cm, r, Wid::StormsOn);
    cm.spawn((Node { flex_grow: 1.0, ..default() }, ChildOf(r)));
    button(cm, r, Wid::StormNow, "Storm now", None);
    dim(cm, root, "(Storm now: a mature cell upwind that drifts over you)");
    cm.spawn_scene(label_dim(String::new())).insert((Txt::Storm, ChildOf(root)));
    // view
    heading(cm, root, "View");
    let r = row(cm, root);
    lbl(cm, r, "Clouds");
    toggle(cm, r, Wid::Clouds);
    lbl(cm, r, "Shadows");
    toggle(cm, r, Wid::Shadows);
    let r = row(cm, root);
    lbl(cm, r, "Debug view");
    select(cm, r, Wid::Debug, &c::DEBUG_NAMES, dash.0.debug as usize);
    // A/B the cloud resolve live (1 = every pixel marched every frame, the reference; 4 = default). Synced to the real value on open.
    let r = row(cm, root);
    lbl(cm, r, "Cloud update div");
    select(cm, r, Wid::UpdateDiv, &["1 (full, slow)", "2", "4 (default)"], 2);
    // map
    heading(cm, root, "Map");
    let r = row(cm, root);
    lbl(cm, r, "Channel");
    select(cm, r, Wid::Channel, &Channel::ALL.map(Channel::label), dash.0.channel.index());
    let m = cm.spawn((MapView, Node { width: px(MAP_W), height: px(map_h), flex_shrink: 0.0, ..default() }, ChildOf(root))).id();
    cm.spawn((ImageNode::new(handle), Node { position_type: PositionType::Absolute, width: px(MAP_W), height: px(map_h), ..default() }, Pickable::IGNORE, ChildOf(m)));
    // storm marks first, so the camera mark draws on top of them
    for i in 0..nephthene_core::storm::MAX_CELLS {
        cm.spawn((Mark::Storm(i), Node { position_type: PositionType::Absolute, display: Display::None, ..default() }, BackgroundColor(Color::srgb(0.55, 0.35, 0.95)), Pickable::IGNORE, ChildOf(m)));
    }
    for (k, color, border) in [(Mark::Pin, Color::srgb(1.0, 0.9, 0.2), false), (Mark::Heading, Color::srgb(1.0, 0.25, 0.2), false), (Mark::Cam, Color::srgb(1.0, 0.25, 0.2), false)] {
        let n = Node { position_type: PositionType::Absolute, border: if border { UiRect::all(px(2.0)) } else { default() }, ..default() };
        if border { cm.spawn((k, n, BorderColor::all(color), Pickable::IGNORE, ChildOf(m))); } else { cm.spawn((k, n, BackgroundColor(color), Pickable::IGNORE, ChildOf(m))); }
    }
    dim(cm, root, "red dot = you (bar = heading), yellow = pinned, violet = storms. north up.");
    cm.spawn_scene(label("hover the map".to_string())).insert((Txt::Hover, ChildOf(root)));
    cm.spawn_scene(label("click the map to pin a point".to_string())).insert((Txt::Pin, ChildOf(root)));
    // gizmos: the legend
    heading(cm, root, "Gizmos (in the world)");
    for (i, (name, rgb)) in c::GIZMOS.iter().enumerate() {
        let r = row(cm, root);
        cm.spawn((Node { width: px(26.0), height: px(10.0), border_radius: BorderRadius::all(px(3.0)), ..default() }, BackgroundColor(col(*rgb)), ChildOf(r)));
        checkbox(cm, r, Wid::Giz(i), name);
    }
    // readout
    heading(cm, root, "Here");
    cm.spawn_scene(label(String::new())).insert((Txt::Here, ChildOf(root)));
    cm.spawn_scene(label(String::new())).insert((Txt::Air, ChildOf(root)));
    cm.spawn_scene(label_dim(String::new())).insert((Txt::Climate, ChildOf(root)));
    cm.spawn_scene(label_dim(String::new())).insert((Txt::Oro, ChildOf(root)));
    cm.spawn_scene(label_dim(String::new())).insert((Txt::Rain, ChildOf(root)));
    cm.spawn_scene(label(String::new())).insert((Txt::Sun, ChildOf(root)));
    cm.spawn_scene(label_dim(String::new())).insert((Txt::Frame, ChildOf(root)));
}

fn input(key: Option<Res<ButtonInput<KeyCode>>>, mut d: ResMut<Panel>) {
    if key.is_some_and(|k| k.just_pressed(KeyCode::Backquote)) { d.0.open = !d.0.open; }
}

// ---- events from the widgets ----
fn kind(q: &Query<&Wid>, e: Entity) -> Option<Wid> { q.get(e).ok().copied() }

fn on_activate(ev: On<Activate>, q: Query<&Wid>, mut d: ResMut<Panel>, mut wx: ResMut<Weather>, storms: Option<ResMut<crate::storms::Storms>>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>) {
    let base = wx.cfg.clock.minutes_per_second / 60.0;
    match kind(&q, ev.entity) {
        Some(Wid::Close) => d.0.open = false,
        Some(Wid::PlayPause) => wx.rate = if wx.rate > 0.0 { 0.0 } else { base },
        Some(Wid::Reset) => d.0.reset_weather(),
        Some(Wid::StormNow) => if let Some(mut s) = storms && let Some(p) = cam.iter().next().map(|g| g.translation()) { s.cfg.enabled = true; s.summon(&wx, p); },
        _ => {}
    }
}

fn on_f32(ev: On<ValueChange<f32>>, q: Query<&Wid>, mut d: ResMut<Panel>, mut wx: ResMut<Weather>) {
    let v = ev.value;
    match kind(&q, ev.source) {
        Some(Wid::Hour) => wx.hour = v.rem_euclid(24.0),
        Some(Wid::Cov) => d.0.coverage = Some(v.clamp(0.0, 1.0)),
        Some(Wid::WindDir) => d.0.wind_from = Some(c::wrap_deg(v)),
        Some(Wid::WindSpeed) => d.0.wind_speed = Some(c::clamp_speed(v)),
        Some(Wid::FogAmount) => wx.cfg.ground_fog.force = Some(v.clamp(0.0, 1.0)),
        Some(Wid::FogBurn) => wx.cfg.ground_fog.burn_per_h = v,
        Some(Wid::FogDepth) => wx.cfg.ground_fog.depth_max_m = v,
        Some(Wid::FogVis) => wx.cfg.ground_fog.visibility_m = v,
        Some(Wid::FogWisp) => wx.cfg.ground_fog.wisp_m = v,
        _ => {}
    }
}

fn on_bool(ev: On<ValueChange<bool>>, q: Query<&Wid>, mut d: ResMut<Panel>, mut wx: ResMut<Weather>, storms: Option<ResMut<crate::storms::Storms>>) {
    let v = ev.value;
    match kind(&q, ev.source) {
        Some(Wid::Rain) => d.0.rain = v,
        Some(Wid::FogOn) => wx.cfg.ground_fog.enabled = v,
        Some(Wid::StormsOn) => if let Some(mut s) = storms { s.cfg.enabled = v; },
        Some(Wid::Clouds) => d.0.clouds = v,
        Some(Wid::Shadows) => d.0.shadows = v,
        Some(Wid::Giz(i)) => d.0.gizmos[i] = v,
        _ => {}
    }
}

/// Forced fog amount when "force" is picked on a clear (or nearly clear) model: a visible start, the user drags from there.
/// ponytail: a panel start value, not a look; move it to Config if the panel ever needs another default.
const FOG_FORCE_START: f32 = 0.9;
const FOG_CLEAR: f32 = 0.01;

fn on_select(ev: On<ValueChange<Entity>>, q: Query<&Wid>, idx: Query<&OptionIndex>, mut d: ResMut<Panel>, mut wx: ResMut<Weather>, params: Option<ResMut<CloudParams>>, mut shown: Query<&mut SelShown>, fog: Res<crate::fog::FogState>) {
    let (Some(k), Ok(i)) = (kind(&q, ev.source), idx.get(ev.value).map(|i| i.0)) else { return };
    if let Ok(mut s) = shown.get_mut(ev.source) { s.0 = i; }
    let base = wx.cfg.clock.minutes_per_second / 60.0;
    match k {
        Wid::Rate => wx.rate = c::RATES[i.min(3)] * base,
        Wid::CovMode => d.0.coverage = if i == 0 { None } else { Some(d.0.coverage.unwrap_or(wx.here.coverage)) },
        Wid::FogMode => wx.cfg.ground_fog.force = if i == 0 { None } else { Some(if fog.amount < FOG_CLEAR { FOG_FORCE_START } else { fog.amount }) },
        Wid::Debug => d.0.debug = i as u32,
        Wid::Channel => d.0.channel = Channel::ALL[i.min(4)],
        Wid::UpdateDiv => if let Some(mut p) = params { p.update_div = [1, 2, 4][i.min(2)]; },
        _ => {}
    }
}

fn on_map_move(ev: On<PointerMove>, v: Query<(), With<MapView>>, mut h: ResMut<MapHover>) {
    if v.contains(ev.entity) || ev.hit.position.is_some() && v.contains(ev.event_target()) { h.0 = ev.hit.position.map(|p| Vec2::new(p.x + 0.5, p.y + 0.5)); }
}
fn on_map_out(ev: On<PointerOut>, v: Query<(), With<MapView>>, mut h: ResMut<MapHover>) { if v.contains(ev.entity) { h.0 = None; } }
fn on_map_click(ev: On<PointerClick>, v: Query<(), With<MapView>>, mut d: ResMut<Panel>, wx: Res<Weather>) {
    if !v.contains(ev.entity) { return; }
    let Some(p) = ev.hit.position else { return };
    let m = &wx.map;
    d.0.pin = Some([m.min_x + (p.x + 0.5) * m.size_x, m.max_z - (p.y + 0.5) * m.size_z]);
}

/// Push the overrides into the weather state and the cloud uniforms. The first frame adopts the config-set values (coverage_override,
/// debug_view) so merely loading the panel changes nothing.
fn apply(mut d: ResMut<Panel>, mut wx: ResMut<Weather>, mut rain: ResMut<RainState>, params: Option<ResMut<CloudParams>>, storms: Option<ResMut<crate::storms::Storms>>, time: Res<Time>, mut init: Local<bool>) {
    if let Some(mut s) = storms { s.hide = !d.0.clouds; }
    if let Some(mut p) = params {
        if !*init {
            d.0.coverage = (p.gpu.coverage_override >= 0.0).then_some(p.gpu.coverage_override);
            d.0.debug = p.gpu.debug;
            *init = true;
        }
        p.gpu.coverage_override = d.0.gpu_coverage();
        p.gpu.precip_override = d.0.gpu_precip();
        p.gpu.debug = d.0.debug;
        p.shadow_gpu.strength = d.0.shadow_strength(wx.cfg.clouds.shadow_strength);
    }
    if d.0.wind_from.is_some() || d.0.wind_speed.is_some() {
        wx.wind = w::wind(&c::wind_with(&wx.cfg.wind, d.0.wind_from, d.0.wind_speed), time.elapsed_secs());
    }
    if let Some(s) = d.0.wind_speed { wx.wind_mean_mps = s; }
    if d.0.rain { wx.here.precip = c::RAIN_PRECIP; }
    rain.panel = c::precip_override(d.0.rain);
}

/// State -> widgets, whenever they differ (sliders, checks, number inputs, selects, the shown map channel).
#[allow(clippy::type_complexity)]
fn sync_widgets(
    mut commands: Commands, d: Res<Panel>, wx: Res<Weather>, mut panels: Query<&mut Node, With<DashPanel>>,
    sliders: Query<(Entity, &Wid, &SliderValue)>, checks: Query<(Entity, &Wid, Has<Checked>), With<Checkbox>>,
    nums: Query<(Entity, &Wid, &NumberInputValue)>, sels: Query<(Entity, &Wid, &SelShown)>, rows: Query<(Entity, &OptionIndex)>, kids: Query<&Children>,
    params: Option<Res<CloudParams>>, map_img: Option<Res<MapImage>>, mut images: ResMut<Assets<Image>>, mut shown_map: Local<Option<(Channel, Option<f32>, bool, bool)>>, fog: Res<crate::fog::FogState>,
    storms: Option<Res<crate::storms::Storms>>,
) {
    for mut n in &mut panels { let want = if d.0.open { Display::Flex } else { Display::None }; if n.display != want { n.display = want; } }
    if !d.0.open { return; }
    let base = wx.cfg.clock.minutes_per_second / 60.0;
    let g = &wx.cfg.ground_fog;
    for (e, k, v) in &sliders {
        let want = match k {
            Wid::Hour => wx.hour, Wid::Cov => d.0.coverage.unwrap_or(wx.here.coverage), Wid::FogAmount => g.force.unwrap_or(fog.amount),
            Wid::FogBurn => g.burn_per_h, Wid::FogDepth => g.depth_max_m, Wid::FogVis => g.visibility_m, Wid::FogWisp => g.wisp_m, _ => continue,
        };
        if (v.0 - want).abs() > 1e-4 { commands.entity(e).insert(SliderValue(want)); }
    }
    for (e, k, on) in &checks {
        let want = match k { Wid::Rain => d.0.rain, Wid::FogOn => g.enabled, Wid::StormsOn => storms.as_ref().is_some_and(|s| s.cfg.enabled), Wid::Clouds => d.0.clouds, Wid::Shadows => d.0.shadows, Wid::Giz(i) => d.0.gizmos[*i], _ => continue };
        if want && !on { commands.entity(e).insert(Checked); } else if !want && on { commands.entity(e).remove::<Checked>(); }
    }
    for (e, k, v) in &nums {
        let want = match k { Wid::WindDir => d.0.wind_from.unwrap_or(wx.cfg.wind.from_deg), Wid::WindSpeed => d.0.wind_speed.unwrap_or(wx.cfg.wind.speed_mps), _ => continue };
        if *v != NumberInputValue::F32(want) { commands.entity(e).insert(NumberInputValue::F32(want)); }
    }
    for (e, k, shown) in &sels {
        let want = match k {
            Wid::Rate => c::rate_index(if base > 0.0 { wx.rate / base } else { 0.0 }), Wid::CovMode => d.0.coverage.is_some() as usize,
            Wid::FogMode => g.force.is_some() as usize,
            Wid::Debug => d.0.debug as usize, Wid::UpdateDiv => params.as_ref().map_or(2, |p| match p.update_div { 1 => 0, 2 => 1, _ => 2 }), Wid::Channel => d.0.channel.index(), _ => continue,
        };
        if shown.0 == want { continue; }
        let row = kids.iter_descendants(e).find(|x| rows.get(*x).is_ok_and(|(_, i)| i.0 == want));
        if let Some(row) = row { commands.trigger(SetSelected { entity: e, row }); commands.entity(e).insert(SelShown(want)); }
    }
    // The image shows the overrides the shader uses (coverage, rain, clouds), so it is rebuilt when any of them changes.
    let key = (d.0.channel, d.0.coverage, d.0.rain, d.0.clouds);
    if *shown_map != Some(key) && let Some(m) = &map_img && let Some(mut i) = images.get_mut(&m.0) {
        i.data = Some(c::effective_rgba(&wx.map, &d.0));
        *shown_map = Some(key);
    }
}

fn cam_pose(cam: &Query<&GlobalTransform, With<AtmosphereSettings>>) -> Option<(Vec3, Vec3)> { cam.iter().next().map(|g| (g.translation(), g.forward().as_vec3())) }

/// Distance (m) from `p` to the segment `a`..`b`; a == b is a point.
fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// The storms readout: how many are selected, the nearest cell's edge (km, stage) from the camera in the storm frame, and the storm
/// weather here. `None` (no resource) reads as off.
fn storm_txt(storms: Option<&crate::storms::Storms>, wx: &Weather, p: Vec3) -> String {
    let l = &wx.storm;
    let here = format!("here: rain {:.2}  cover {:.2}  sky {:.2}  gust {:.1} m/s", l.precip, l.cover, l.ambient, l.outflow.length());
    let Some(s) = storms.filter(|s| s.cfg.enabled) else { return "storms off".into() };
    let [sx, sz] = wx.cfg.map.world_offset_m;
    let q = Vec2::new(p.x + sx, p.z + sz);
    let near = s.sel.iter().map(|(_, n)| ((seg_dist(q, n.a, n.b) - n.core_r).max(0.0), n.stage)).min_by(|x, y| x.0.total_cmp(&y.0));
    match near {
        Some((d, stage)) => format!("storms {}   nearest {:.1} km ({stage:?})   {here}", s.sel.len(), d / 1000.0),
        None => format!("storms 0   {here}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn refresh_ui(
    d: Res<Panel>, wx: Res<Weather>, rain: Res<RainState>, fog: Res<crate::fog::FogState>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>, time: Res<Time>, hover: Res<MapHover>,
    mut ms: Local<f32>, mut texts: Query<(&Txt, &mut Text)>, mut marks: Query<(&Mark, &mut Node)>, storms: Option<Res<crate::storms::Storms>>,
    oro: Option<Res<crate::orographic::Orographic>>,
) {
    *ms += (time.delta_secs() * 1000.0 - *ms) * 0.1;
    if !d.0.open { return; }
    let ctl = &d.0;
    let (p, fwd) = cam_pose(&cam).unwrap_or((Vec3::ZERO, Vec3::Z));
    // The camera in the map frame: with map.advect the weather moves downwind, so the marker drifts upwind across the map.
    let [gx, gz] = w::map_xz(&wx.cfg.map, wx.map_shift, p.x, p.z);
    let map = &wx.map;
    let (el, az) = w::sun(&wx.cfg.clock, wx.hour);
    let pin_txt = match ctl.pin {
        Some([x, z]) => { let [i, j] = c::texel_at(map, x, z);
            format!("pinned ({x:.0}, {z:.0}) m: {}", ctl.channel.read(c::effective_texel(map, ctl, i, j))) }
        None => "click the map to pin a point".into(),
    };
    let hover_txt = match hover.0 {
        Some(uv) => { let (x, z) = (map.min_x + uv.x * map.size_x, map.max_z - uv.y * map.size_z); let [i, j] = c::texel_at(map, x, z);
            format!("({x:.0}, {z:.0}) m: {}", ctl.channel.read(c::effective_texel(map, ctl, i, j))) }
        None => "hover the map".into(),
    };
    for (k, mut t) in &mut texts {
        let s = match k {
            Txt::Time => format!("{}   ({:.2} h)", c::hhmm(wx.hour), wx.hour),
            Txt::Play => (if wx.rate > 0.0 { "Pause" } else { "Play" }).to_string(),
                        Txt::Here => format!("coverage {:.2}   rain {:.2}   dust {:.2}", wx.here.coverage, wx.here.precip, wx.here.dust),
            Txt::Air => { let a = &wx.air; format!("air {:.1} C   dew {:.1} C   rh {:.2}   freezing {:.0} m   {}", a.temp_c, a.dew_c, a.rh, a.freezing_m, c::precip_word(a.precip_kind)) }
            Txt::Climate => { let k = &wx.climate_here; format!("climate {:+.1} K   humidity {:+.2}   snow {:.2}   ground wet {:.2}", k.temp_k, k.humidity, k.snow, k.wetness) }
            Txt::Oro => match oro.as_ref().and_then(|o| o.tile.as_ref()) {
                Some(t) => { let [pr, cv] = t.sample(p.x, p.z); format!("terrain uplift: precip {pr:.2}   cover {cv:.2}") }
                None => "terrain uplift: off".into(),
            },
            Txt::Rain => format!("rain: intensity {:.2}   wetness {:.2}   fall {:.1} m/s   slant {:.0} deg   source {}", rain.intensity, rain.wetness, -rain.vel.y, rain.slant_deg(), rain.source()),
            Txt::Sun => format!("sun elevation {el:.1} deg   azimuth {az:.1} deg   heading {:.0}", c::heading_deg(fwd.to_array())),
            Txt::Frame => format!("frame {:.1} ms ({:.0} fps)", *ms, 1000.0 / ms.max(0.1)),
            Txt::Fog => format!("fog amount {:.2}   lift {:.0} m", fog.amount, fog.lift),
            Txt::Storm => storm_txt(storms.as_deref(), &wx, p),
            Txt::Hover => hover_txt.clone(),
            Txt::Pin => pin_txt.clone(),
        };
        if t.0 != s { t.0 = s; }
    }
    let map_h = MAP_W * map.height as f32 / map.width as f32;
    let at = |x: f32, z: f32| { let [u, v] = c::map_uv(map, x, z); Vec2::new(u * MAP_W, v * map_h) };
    let rect = |n: &mut Node, centre: Vec2, size: Vec2| { (n.left, n.top, n.width, n.height) = (px(centre.x - size.x / 2.0), px(centre.y - size.y / 2.0), px(size.x), px(size.y)); };
    let cam_px = at(gx, gz);
    for (m, mut n) in &mut marks {
        match m {
            Mark::Cam => rect(&mut n, cam_px, Vec2::splat(9.0)),
            Mark::Heading => { let a = c::heading_deg(fwd.to_array()).to_radians(); rect(&mut n, cam_px + Vec2::new(a.sin(), -a.cos()) * 14.0, Vec2::splat(5.0)); }
            Mark::Pin => match ctl.pin { Some([x, z]) => { n.display = Display::Flex; rect(&mut n, at(x, z), Vec2::splat(9.0)); } None => n.display = Display::None },
            // the storm's axis middle: map frame, back to world, then to the shown map as the camera is
            Mark::Storm(i) => match storms.as_ref().and_then(|s| s.sel.get(*i)) {
                Some((_, s)) => {
                    let c = 0.5 * (s.a + s.b);
                    let [gx, gz] = w::map_xz(&wx.cfg.map, wx.map_shift, c.x - wx.cfg.map.world_offset_m[0], c.y - wx.cfg.map.world_offset_m[1]);
                    n.display = Display::Flex; rect(&mut n, at(gx, gz), Vec2::splat(11.0));
                }
                None => n.display = Display::None,
            },
        }
    }
}

/// Mouse wheel scrolls the panel while the cursor is over it (it is taller than short windows).
fn scroll(mut wheel: MessageReader<MouseWheel>, d: Res<Panel>, mut panel: Query<(&RelativeCursorPosition, &mut ScrollPosition), With<DashPanel>>) {
    let dy: f32 = wheel.read().map(|e| match e.unit { MouseScrollUnit::Line => e.y * 28.0, MouseScrollUnit::Pixel => e.y }).sum();
    if !d.0.open || dy == 0.0 { return; }
    for (r, mut s) in &mut panel { if r.cursor_over { s.y = (s.y - dy).max(0.0); } }
}

fn draw_gizmos(mut g: Gizmos, d: Res<Panel>, wx: Res<Weather>, params: Option<Res<CloudParams>>, cam: Query<&GlobalTransform, With<AtmosphereSettings>>) {
    if !d.0.open { return; }
    let Some((p, _)) = cam_pose(&cam) else { return };
    let ground = Vec3::new(p.x, 0.0, p.z);
    let (el, az) = w::sun(&wx.cfg.clock, wx.hour);
    let sun = w::dir(el, az);
    if d.0.gizmos[0] {
        let k = col(c::GIZMOS[0].1);
        g.arrow(ground + Vec3::Y * 2.0, ground + Vec3::Y * 2.0 + sun * 1500.0, k);
        g.line(ground, ground + Vec3::Y * 2.0, k);
    }
    if d.0.gizmos[1] && wx.wind.length() > 0.01 {
        let dir = wx.wind.normalize();
        g.arrow(ground + Vec3::Y * 10.0, ground + Vec3::Y * 10.0 + dir * 15.0 * wx.wind.length(), col(c::GIZMOS[1].1));
    }
    if d.0.gizmos[2] && let Some(cp) = &params && sun.y > 0.02 {
        let (s, k) = (&cp.shadow_gpu, col(c::GIZMOS[2].1));
        let cs: Vec<Vec3> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].into_iter()
            .map(|l| w::shadow_ground(s.origin, s.right, s.up, sun, s.half_m, l) + Vec3::Y * 0.3).collect();
        for i in 0..4 { g.line(cs[i], cs[(i + 1) % 4], k); }
        let mid = w::shadow_ground(s.origin, s.right, s.up, sun, s.half_m, [0.0, 0.0]) + Vec3::Y * 0.3;
        g.cross(Isometry3d::from_translation(mid), 200.0, k);
    }
}
