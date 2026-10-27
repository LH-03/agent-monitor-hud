//! Interactive settings samples use the real HUD builders, never user sessions.

use crate::config::Settings;
use crate::engine::{Notice, View, ledger::LedgerView};
use crate::locale::Catalog;
use crate::ui::{anim, gfx::Gfx, hud, layout, paint::{DrawState, Painter, mul}, style::{self, Role}};
use layout::{El, Rect, Th};
use std::collections::HashSet;
use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows_numerics::Matrix3x2;

pub struct Sample {
    root: El,
    size: (f32, f32),
    animations: Vec<(u64, anim::Anim)>,
    image: Option<String>,
    quiet_expanded: Option<El>,
    quiet_started: i64,
}

impl Sample {
    pub fn build(gfx: &Gfx, catalog: &Catalog, settings: &Settings, kind: u8, width: f32, started: i64, now: i64) -> Self {
        let loc = catalog.get(&settings.language);
        let sloc = catalog.settings_locale(&settings.language);
        let mut task = crate::ui::preview::synthetic(1, now);
        task.workspace = sloc.get("previewProject").into();
        task.title = sloc.get("previewTask").into();
        task.snapshot.as_mut().unwrap().model = "GPT".into();
        let mut second = task.clone();
        second.key = "synthetic-2".into();
        second.number = 2;
        second.status = "waiting";
        second.source = crate::engine::Source::Claude;
        second.client = "claude-desktop";
        second.provider = "anthropic".into();
        let mut display = task.snapshot.clone().unwrap();
        display.claude_five_hour_remaining = Some(22.0);
        display.claude_weekly_remaining = Some(71.0);
        display.dsh_balance = Some("¥23.50".into());
        display.dsh_balance_value = Some(23.5);
        let mut view = View {
            tasks: vec![task.clone(), second.clone()], display: Some(display),
            overall_status: "active", initial_scan_complete: true,
            ledger: LedgerView { today_tokens: 2_413_000, today_cost: Some(18.4), minutes: (0..60).map(|i| (i * 137 % 4000) as i64).collect() },
            ..Default::default()
        };
        if kind == 0 {
            let mut third = task.clone();
            third.key = "synthetic-3".into();
            third.number = 3;
            third.source = crate::engine::Source::Dsh;
            third.client = "dsh-desktop";
            third.provider = "deepseek".into();
            view.tasks.push(third);
        }
        if kind == 3 {
            view.overall_status = "idle";
            for task in &mut view.tasks { task.status = "idle"; task.started_at = now - 3_600_000; }
            for minute in view.ledger.minutes.iter_mut().rev().take(15) { *minute = 0; }
        }
        let image = (settings.theme.surface == "image" && std::path::Path::new(&settings.theme.background_image).is_file()).then(|| settings.theme.background_image.clone());
        let ctx = hud::Ctx { s: settings, loc: &loc, sloc: &sloc, zh: &catalog.zh, en: &catalog.en, now, image_available: image.is_some() };
        let mut actions = hud::ActionMap::new();
        let mut root = match kind {
            1 => El::panel().m(Th::all(12.0)).child(El::border(
                style::surface(settings, "active", false, image.is_some()),
                style::solid(&settings.border, "#22FFFFFF", Role::Decoration, settings, "active", false),
                settings.theme.border_width as f32, settings.corner_radius as f32,
            ).p(hud::expanded_padding(settings, (settings.main.spread_x as f32, settings.main.spread_y as f32)))
                .child(hud::task_list(&ctx, &view.tasks, &HashSet::new(), settings.main.spread_y as f32, &mut actions))),
            2 => hud::build_bubble(&ctx, &task, false, None),
            4 if settings.notices.enabled => {
                task.notice = Some(Notice { text: sloc.get("previewMessage").into(), until: now + 30_000, recipe: None, agent: task.source, revision: 1 });
                hud::build_notice(&ctx, &task).unwrap()
            }
            4 => super::widgets::wrap(sloc.get("previewNoticeOff"), 12.0, super::widgets::MUTED).m(Th::all(14.0)),
            _ => hud::build_main(&ctx, &hud::MainModel {
                view: &view, detached: &HashSet::new(), list_visible: false, quiet: kind == 3,
                overall: view.overall_status, show_grip: false, spread: (settings.main.spread_x as f32, settings.main.spread_y as f32),
            }, &mut actions),
        };
        if settings.transparency_mode == "uniform" && kind != 4 {
            root.opacity = settings.opacity as f32;
        }
        let size = layout::layout_root(&mut root, (width, f32::INFINITY), &layout::Ctx { gfx, scale: 1.0 });
        let quiet_expanded = (kind == 3).then(|| {
            let mut expanded = hud::build_main(&ctx, &hud::MainModel {
                view: &view, detached: &HashSet::new(), list_visible: false, quiet: false, overall: "idle", show_grip: false,
                spread: (settings.main.spread_x as f32, settings.main.spread_y as f32),
            }, &mut actions);
            expanded.opacity = root.opacity;
            layout::layout_root(&mut expanded, (width, f32::INFINITY), &layout::Ctx { gfx, scale: 1.0 });
            expanded
        });
        let mut animations = Vec::new();
        if started > 0 {
            let start = started as f64;
            let until = start + settings.attention.duration_seconds as f64 * 1000.0;
            let (surface, dot, mode) = match kind {
                1 => ("row.surface:synthetic-1", "row.dot:synthetic-1", &settings.attention.list_mode),
                2 => ("bubble.shell:synthetic-1", "bubble.dot:synthetic-1", &settings.attention.bubble_mode),
                _ => ("main.shell", "main.dot", &settings.attention.summary_mode),
            };
            if kind == 4 && settings.notices.enabled {
                animations.push((hud::fx_key("notice.card"), anim::agent(settings, None, start, start + settings.notices.duration_seconds as f64 * 1000.0)));
            } else if kind < 3 {
                if let Some(a) = anim::surface(settings, mode, start, until) { animations.push((hud::fx_key(surface), a)); }
                if (kind != 0 || settings.show_status_dot) && let Some(a) = anim::dot(settings, &settings.status_color("active"), start, until) {
                    animations.push((hud::fx_key(dot), a));
                }
            }
        }
        Self { root, size, animations, image, quiet_expanded, quiet_started: if settings.animate_updates { started } else { 0 } }
    }

    pub fn moving(&self, now: i64) -> bool {
        self.animations.iter().any(|(_, a)| !a.finished(now as f64)) || (self.quiet_expanded.is_some() && self.quiet_started > 0 && now - self.quiet_started < 1600)
    }

    pub fn draw(&self, p: &mut Painter, gfx: &Gfx, rect: Rect, now: i64) {
        p.images.retain(|key, _| self.image.as_ref() == Some(key));
        if let Some(key) = &self.image {
            if !p.images.contains_key(key) && let Some(bitmap) = crate::ui::image::load(&p.target, key) {
                p.images.insert(key.clone(), bitmap);
            }
        }
        let elapsed = if self.quiet_started > 0 { now - self.quiet_started } else { 1600 };
        let mut motion = None;
        let mut shown = &self.root;
        if let Some(expanded) = &self.quiet_expanded {
            if elapsed < 320 { motion = Some((&self.root, expanded, elapsed as f32 / 320.0)); }
            else if elapsed < 1000 { shown = expanded; }
            else if elapsed < 1280 { motion = Some((expanded, &self.root, (elapsed - 1000) as f32 / 280.0)); }
        }
        let display_size = motion.map(|(old, new, t)| crate::ui::morph::blend(
            crate::ui::morph::Frame { size: old.desired, pos: (0.0, 0.0) }, crate::ui::morph::Frame { size: new.desired, pos: (0.0, 0.0) }, t).size).unwrap_or(shown.desired);
        let natural = self.quiet_expanded.as_ref().map(|r| r.desired).unwrap_or(self.size);
        let scale = (rect.w / natural.0.max(1.0)).min(rect.h / natural.1.max(1.0)).min(1.0);
        let transform = Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale,
            M31: rect.x + (rect.w - display_size.0 * scale) / 2.0, M32: rect.y + (rect.h - display_size.1 * scale) / 2.0 };
        let fx = |key| self.animations.iter().find(|(id, a)| *id == key && !a.finished(now as f64)).map(|(_, a)| a.fx(now as f64));
        let custom = |_: &mut Painter, _: u32, _: Rect| {};
        let state = DrawState { hovered: 0, pressed: 0, fx: &fx, custom: &custom };
        unsafe {
            let mut old = Matrix3x2::default();
            p.target.GetTransform(&mut old);
            p.target.PushAxisAlignedClip(&D2D_RECT_F { left: rect.x, top: rect.y, right: rect.x + rect.w, bottom: rect.y + rect.h }, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            p.target.SetTransform(&mul(&transform, &old));
            if let Some((from, to, t)) = motion {
                let offset = |root: &El| ((display_size.0 - root.desired.0) / 2.0, (display_size.1 - root.desired.1) / 2.0);
                crate::ui::morph::draw(p, gfx, from, to, display_size, offset(from), offset(to), t, crate::ui::morph::radius(from), 1.0, &state);
            } else { p.draw(shown, gfx, &state); }
            p.target.SetTransform(&old);
            p.target.PopAxisAlignedClip();
        }
    }
}
