use std::{cell::RefCell, rc::Rc};

use gpui::{TestAppContext, size};
use magenta_core::{EffortLevel, GenerationConfig, ModelDescriptor};

use super::{PromptComposer, PromptComposerEvent, is_supported_image};

fn model(id: &str, default_effort: EffortLevel) -> ModelDescriptor {
    ModelDescriptor {
        provider: magenta_core::ProviderId::new("openai"),
        id: magenta_core::ModelId::new(id),
        display_name: id.to_owned(),
        description: None,
        priority: 0,
        default_effort,
        supported_efforts: EffortLevel::ALL.to_vec(),
    }
}

#[gpui::test]
fn cancel_is_emitted_only_while_a_response_is_generating(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(gpui::px(720.), gpui::px(420.)), PromptComposer::new);
    let cancellations = Rc::new(RefCell::new(0));
    let observed = Rc::clone(&cancellations);

    let subscription = window
        .update(cx, |composer, _, cx| {
            let subscription = cx.subscribe(&cx.entity(), move |_, _, event, _| {
                if matches!(event, PromptComposerEvent::Cancel) {
                    *observed.borrow_mut() += 1;
                }
            });
            composer.cancel(cx);
            composer.set_generating(true, cx);
            composer.cancel(cx);
            subscription
        })
        .expect("the composer test window should remain open");

    cx.run_until_parked();
    assert_eq!(*cancellations.borrow(), 1);
    drop(subscription);
}

#[gpui::test]
fn pending_storage_blocks_submit_and_preserves_newer_draft(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(gpui::px(900.), gpui::px(640.)), PromptComposer::new);
    window
        .update(cx, |composer, window, cx| {
            composer.select_model(model("model", EffortLevel::Medium), cx);
            composer
                .input
                .update(cx, |input, cx| input.set_value("First draft", window, cx));
            let submitted = composer.request(cx).unwrap();
            composer.set_storage_ready(false, cx);
            assert!(composer.request(cx).is_none());
            composer
                .input
                .update(cx, |input, cx| input.set_value("Newer draft", window, cx));
            composer.clear_submitted(&submitted, window, cx);
            assert_eq!(composer.input.read(cx).value().as_ref(), "Newer draft");
            composer.set_storage_ready(true, cx);
            let submitted = composer.request(cx).unwrap();
            composer.clear_submitted(&submitted, window, cx);
            assert!(composer.input.read(cx).value().is_empty());
        })
        .unwrap();
}

#[test]
fn supported_image_extensions_are_case_insensitive() {
    assert!(is_supported_image(std::path::Path::new("reference.PNG")));
    assert!(is_supported_image(std::path::Path::new("reference.jpeg")));
    assert!(is_supported_image(std::path::Path::new("reference.WebP")));
    assert!(!is_supported_image(std::path::Path::new("reference.gif")));
    assert!(!is_supported_image(std::path::Path::new("reference")));
}

#[gpui::test]
fn composer_requires_content_model_and_effort(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(gpui::px(720.), gpui::px(420.)), PromptComposer::new);

    window
        .update(cx, |composer, window, cx| {
            assert!(!composer.is_ready(cx));
            composer
                .input
                .update(cx, |input, cx| input.set_value("   ", window, cx));
            let model = model("gpt-5.4", EffortLevel::Medium);
            composer.set_models(vec![model.clone()], cx);
            composer.select_model(model, cx);
            composer.select_effort(EffortLevel::Medium, cx);
            assert!(!composer.is_ready(cx));
            composer.input.update(cx, |input, cx| {
                input.set_value("A luminous glass sphere", window, cx);
            });
            assert!(composer.is_ready(cx));
        })
        .expect("the composer test window should remain open");
}

#[gpui::test]
fn request_trims_prompt_and_preserves_configuration(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(gpui::px(720.), gpui::px(420.)), PromptComposer::new);

    window
        .update(cx, |composer, window, cx| {
            composer.input.update(cx, |input, cx| {
                input.set_value("  A quiet cyan horizon  ", window, cx);
            });
            let model = model("gpt-5.4", EffortLevel::High);
            composer.set_models(vec![model.clone()], cx);
            composer.select_model(model, cx);
            composer.select_effort(EffortLevel::High, cx);

            let request = composer.request(cx).expect("the request should be ready");
            assert_eq!(request.prompt.as_ref(), "A quiet cyan horizon");
            assert_eq!(
                request.generation.provider,
                magenta_core::ProviderId::new("openai")
            );
            assert_eq!(
                request.generation.model,
                magenta_core::ModelId::new("gpt-5.4")
            );
            assert_eq!(request.generation.effort, EffortLevel::High);
            assert!(request.attachments.is_empty());
        })
        .expect("the composer test window should remain open");
}

#[test]
fn model_options_round_trip_through_core_generation_configuration() {
    let model = model("gpt-5.4", EffortLevel::Medium);
    let configuration =
        GenerationConfig::new(model.provider.clone(), model.id, EffortLevel::Medium);

    assert_eq!(
        configuration.provider,
        magenta_core::ProviderId::new("openai")
    );
    assert_eq!(configuration.model, magenta_core::ModelId::new("gpt-5.4"));
    assert_eq!(configuration.effort, EffortLevel::Medium);
}

#[gpui::test]
fn selecting_a_model_uses_only_its_advertised_efforts(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(gpui::px(720.), gpui::px(420.)), PromptComposer::new);
    let openai = ModelDescriptor {
        supported_efforts: vec![EffortLevel::Low, EffortLevel::Medium, EffortLevel::XHigh],
        default_effort: EffortLevel::XHigh,
        ..model("gpt-5.6-luna", EffortLevel::Medium)
    };
    let gemini_effort = EffortLevel::from_wire("thinking_budget").unwrap();
    let gemini = ModelDescriptor {
        provider: magenta_core::ProviderId::new("gemini"),
        id: magenta_core::ModelId::new("gemini-2.5-pro"),
        display_name: "Gemini Pro".to_owned(),
        description: None,
        priority: 0,
        default_effort: gemini_effort.clone(),
        supported_efforts: vec![gemini_effort.clone()],
    };

    window
        .update(cx, |composer, _, cx| {
            composer.set_models(vec![openai.clone(), gemini.clone()], cx);
            composer.select_model(openai, cx);
            composer.select_effort(EffortLevel::XHigh, cx);
            assert_eq!(composer.effort, Some(EffortLevel::XHigh));
            composer.select_model(gemini, cx);
            assert_eq!(composer.effort, Some(gemini_effort));
        })
        .expect("the composer test window should remain open");
}

#[gpui::test]
fn core_configuration_restores_the_composer_selection(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(gpui::px(720.), gpui::px(420.)), PromptComposer::new);
    let configuration = GenerationConfig::new(
        magenta_core::ProviderId::new("openai"),
        magenta_core::ModelId::new("gpt-5.4"),
        EffortLevel::High,
    );

    window
        .update(cx, |composer, _, cx| {
            composer.set_models(vec![model("gpt-5.4", EffortLevel::Medium)], cx);
            composer.set_configuration(&configuration, cx);
            assert_eq!(
                composer.model.as_ref().map(|model| &model.id),
                Some(&configuration.model)
            );
            assert_eq!(composer.effort, Some(EffortLevel::High));
        })
        .expect("the composer test window should remain open");
}

#[gpui::test]
fn shift_enter_pairs_a_fence_and_keeps_the_caret_in_the_code_body(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let window = cx.open_window(size(gpui::px(720.), gpui::px(420.)), PromptComposer::new);

    window
        .update(cx, |composer, window, cx| {
            let opening = "```rust\n";
            composer.input.update(cx, |input, cx| {
                input.set_value(opening, window, cx);
                input.set_selected_range(opening.len()..opening.len(), cx);
            });
            composer.handle_shift_enter(window, cx);
            let input = composer.input.read(cx);
            assert_eq!(input.value().as_ref(), "```rust\n\n```");
            assert_eq!(input.cursor(), opening.len());
        })
        .expect("the composer test window should remain open");
}
