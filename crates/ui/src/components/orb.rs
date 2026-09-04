use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ImageCacheError, ObjectFit, RenderImage,
    Transformation, Window, div, img, percentage, prelude::*, px, rgb, size, svg,
};

const ORB_SIZE: f32 = 260.;
const ORB_SVG: &[u8] = include_bytes!("../assets/glowing_orb.svg");
const ORB_WAVE_SVG: &[u8] = include_bytes!("../assets/glowing_orb_wave.svg");
const ORB_RIM_SVG: &[u8] = include_bytes!("../assets/glowing_orb_rim.svg");

static ORB_IMAGE: OnceLock<Result<Arc<RenderImage>, ImageCacheError>> = OnceLock::new();
static ORB_RIM_IMAGE: OnceLock<Result<Arc<RenderImage>, ImageCacheError>> = OnceLock::new();

pub(super) fn render() -> AnyElement {
    div()
        .relative()
        .size(px(ORB_SIZE))
        .child(
            img(orb_image)
                .size_full()
                .object_fit(ObjectFit::Fill)
                .with_fallback(|| {
                    div()
                        .size_full()
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(0x35d6_ffff))
                        .bg(rgb(0x0711_16ff))
                        .into_any_element()
                }),
        )
        .child(wave_layer())
        .child(
            img(orb_rim_image)
                .absolute()
                .top_0()
                .left_0()
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
        .child(
            // GPUI's color image element has no transform primitive. The SVG
            // element does, so use the wave SVG as an alpha mask and rotate
            // only the luminous interior while the glass shell stays still.
            svg()
                .data(ORB_WAVE_SVG)
                .size_full()
                .text_color(rgb(0x8bee_ffff))
                .with_animation(
                    "glowing-orb-ribbon-rotation",
                    Animation::new(Duration::from_secs(14))
                        .repeat()
                        .with_max_fps(60.),
                    |this, progress| {
                        let angle = progress * std::f32::consts::TAU;
                        let depth = 0.42_f32.mul_add(angle.cos().abs(), 0.58);

                        this.with_transformation(
                            Transformation::rotate(percentage(progress))
                                .with_scaling(size(1., depth)),
                        )
                    },
                ),
        )
        .into_any_element()
}

fn orb_image(
    _window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    cached_image(&ORB_IMAGE, ORB_SVG, cx)
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
    cx: &App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    cache.get_or_init(|| {
        cx.svg_renderer()
            .render_single_frame(svg, 1.)
            .map_err(ImageCacheError::from)
    });
    cache.get().cloned()
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
