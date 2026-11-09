use super::*;
use gpui_kit::{Modifiers, ScrollDelta, ScrollEvent, point, px};

#[gpui_kit::test]
fn long_catalog_keeps_new_models_reachable_and_selectable(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(960.), px(720.)), PromptComposer::new);
    let mut catalog = (0..16)
        .map(|index| model(&format!("catalog-model-{index}"), EffortLevel::Medium))
        .collect::<Vec<_>>();
    catalog.push(ModelDescriptor {
        supported_efforts: vec![EffortLevel::Low, EffortLevel::High],
        ..model("gpt-6.1-terra", EffortLevel::High)
    });
    window
        .update(cx, |composer, _, cx| composer.set_models(catalog, cx))
        .unwrap();
    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let trigger = visual.debug_bounds("prompt-model-selector").unwrap();
    visual.simulate_click(trigger.center(), Modifiers::default());
    visual.run_until_parked();
    let list = visual.debug_bounds("prompt-model-list").unwrap();
    let surface = visual.debug_bounds("prompt-model-picker-surface").unwrap();
    assert!(surface.size.height <= px(420.));
    let latest = "model-openai-gpt-6.1-terra";
    let before = visual.debug_bounds(latest).unwrap();
    assert!(before.top() >= surface.bottom());
    visual.simulate_event(ScrollEvent {
        position: point(list.center().x, list.top() + px(100.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-1600.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let after = visual.debug_bounds(latest).unwrap();
    assert!(after.top() < before.top());
    assert!(after.center().y >= surface.top() && after.center().y <= surface.bottom());
    visual.simulate_click(after.center(), Modifiers::default());
    visual.run_until_parked();
    window
        .update(cx, |composer, _, _| {
            assert_eq!(
                composer.model.as_ref().map(|model| model.id.0.as_str()),
                Some("gpt-6.1-terra")
            );
            assert_eq!(composer.effort, Some(EffortLevel::High));
            assert_eq!(
                composer
                    .model
                    .as_ref()
                    .map(|model| model.supported_efforts.as_slice()),
                Some([EffortLevel::Low, EffortLevel::High].as_slice())
            );
        })
        .unwrap();
}
