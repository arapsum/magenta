use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, ImageCacheError, ObjectFit,
    RenderImage, Window, div, img, prelude::*, px, rgb,
};

use crate::app::MainView;

const ORB_SIZE: f32 = 260.;
const ORB_SVG: &[u8] = include_bytes!("../assets/glowing_orb.svg");
const ORB_WAVE_SVG: &[u8] = include_bytes!("../assets/glowing_orb_wave.svg");
const ORB_RIM_SVG: &[u8] = include_bytes!("../assets/glowing_orb_rim.svg");

static ORB_IMAGE: OnceLock<Result<Arc<RenderImage>, ImageCacheError>> = OnceLock::new();
static ORB_WAVE_IMAGE: OnceLock<Result<Arc<RenderImage>, ImageCacheError>> = OnceLock::new();
static ORB_RIM_IMAGE: OnceLock<Result<Arc<RenderImage>, ImageCacheError>> = OnceLock::new();

pub(crate) fn render(_cx: &mut Context<MainView>) -> AnyElement {
    div()
        .relative()
        .size(px(ORB_SIZE))
        .with_animation(
            "glowing-orb-motion",
            // The two source animations have different periods. A 35-second
            // shared cycle preserves their 7s float and 5s breathing rhythms.
            Animation::new(Duration::from_secs(35)).repeat(),
            |this, progress| {
                let float_phase = progress * 5. * std::f32::consts::TAU;
                let breath_phase = progress * 7. * std::f32::consts::TAU;
                let vertical_offset = 0.5 - 3.5 * float_phase.cos();
                let opacity = 0.96 + 0.04 * (0.5 - 0.5 * breath_phase.cos());

                this.top(px(vertical_offset)).opacity(opacity)
            },
        )
        .child(
            img(orb_image)
                .size_full()
                .object_fit(ObjectFit::Fill)
                .with_fallback(|| {
                    div()
                        .size_full()
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(0x35d6ffff))
                        .bg(rgb(0x071116ff))
                        .into_any_element()
                }),
        )
        .child(wave_layer())
        .child(
            img(orb_rim_image)
                .size_full()
                .object_fit(ObjectFit::Fill)
                .with_fallback(|| div().size_full().into_any_element()),
        )
        .into_any_element()
}

fn wave_layer() -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(px(ORB_SIZE))
        .with_animation(
            "glowing-orb-ribbon-motion",
            Animation::new(Duration::from_secs_f32(6.5)).repeat(),
            |this, progress| {
                let phase = 0.5 - 0.5 * (progress * std::f32::consts::TAU).cos();
                let scale = 1. - 0.03 * phase;
                let height = ORB_SIZE * scale;
                let top = (ORB_SIZE - height) / 2. - 1. + 3. * phase;

                this.top(px(top)).h(px(height))
            },
        )
        .child(
            img(orb_wave_image)
                .size_full()
                .object_fit(ObjectFit::Fill)
                .with_fallback(|| div().size_full().into_any_element()),
        )
        .into_any_element()
}

fn orb_image(
    _window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    cached_image(&ORB_IMAGE, ORB_SVG, cx)
}

fn orb_wave_image(
    _window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    cached_image(&ORB_WAVE_IMAGE, ORB_WAVE_SVG, cx)
}

fn orb_rim_image(
    _window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    cached_image(&ORB_RIM_IMAGE, ORB_RIM_SVG, cx)
}

fn cached_image(
    cache: &OnceLock<Result<Arc<RenderImage>, ImageCacheError>>,
    svg: &[u8],
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    Some(
        cache
            .get_or_init(|| {
                cx.svg_renderer()
                    .render_single_frame(svg, 1.)
                    .map_err(ImageCacheError::from)
            })
            .clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orb_asset_renders_as_a_high_resolution_frame() {
        let renderer = gpui::SvgRenderer::new(Arc::new(()));
        for svg in [ORB_SVG, ORB_WAVE_SVG, ORB_RIM_SVG] {
            let image = renderer
                .render_single_frame(svg, 1.)
                .expect("the bundled orb SVG should be valid");

            assert_eq!(image.frame_count(), 1);
            assert_eq!(image.size(0).width.0, 600);
            assert_eq!(image.size(0).height.0, 600);
            assert!(image.as_bytes(0).is_some_and(|bytes| {
                bytes
                    .chunks(4)
                    .any(|pixel| pixel.get(3).is_some_and(|alpha| *alpha > 0))
            }));
        }
    }
}
