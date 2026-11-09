use gpui_kit::{Animation, AnimationExt as _};

use super::*;

pub(super) fn render(
    message_id: MessageId,
    label: String,
    progress: Option<&GenerationProgress>,
    cx: &App,
) -> AnyElement {
    let started_at = progress.map(|progress| progress.started_at);
    let foreground = cx.theme().muted_foreground;
    let mono = cx.theme().mono_font_family.clone();
    h_flex()
        .debug_selector(|| "assistant-grid-loader".to_owned())
        .min_w_0()
        .items_center()
        .gap(px(9.))
        .with_animation(
            ("assistant-grid-loader", message_id.0),
            Animation::new(Duration::from_millis(1_350))
                .repeat()
                .with_max_fps(20.),
            move |this, phase| {
                this.child(grid(phase, foreground))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(foreground)
                            .child(label.clone()),
                    )
                    .when_some(started_at, |this, started_at| {
                        this.child(
                            div()
                                .min_w(px(46.))
                                .font_family(mono.clone())
                                .text_size(px(11.))
                                .text_color(foreground)
                                .child(format_timer(started_at.elapsed())),
                        )
                    })
            },
        )
        .into_any_element()
}

fn grid(phase: f32, foreground: gpui_kit::Hsla) -> AnyElement {
    let mut grid = v_flex().flex_none().gap(px(2.)).size(px(13.));
    for row in 0_u8..3 {
        let mut line = h_flex().flex_none().gap(px(2.));
        for column in 0_u8..3 {
            // A soft wave crosses the nine pixels diagonally, like the reference.
            let offset = f32::from(row + column * 2) / 9.;
            let wave = ((phase - offset) * std::f32::consts::TAU)
                .cos()
                .mul_add(0.5, 0.5);
            line = line.child(
                div()
                    .flex_none()
                    .size(px(3.))
                    .rounded(px(0.5))
                    .bg(foreground.opacity(wave.mul_add(0.76, 0.18))),
            );
        }
        grid = grid.child(line);
    }
    grid.into_any_element()
}

pub(super) fn format_timer(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let tenths = elapsed.subsec_millis() / 100;
    if seconds < 60 {
        format!("{seconds}.{tenths}s")
    } else if seconds < 3_600 {
        format!("{}m {}.{tenths}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3_600, seconds / 60 % 60)
    }
}
