use super::*;

impl PromptComposer {
    pub(crate) fn model_selector(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let selected_model = self.model.clone();
        let selected_effort = self.effort.clone();
        let models = self.models.clone();
        let automatic_selected = self.active_generation_is_automatic();
        let model_unavailable =
            self.model_catalog_state.is_loaded() && !self.active_model_available();
        let effort_unavailable =
            self.model_catalog_state.is_loaded() && !self.active_effort_available();
        let efforts = selected_model
            .as_ref()
            .map_or_else(Vec::new, |model| model.supported_efforts.clone());
        let model_label: SharedString = selected_model.as_ref().map_or_else(
            || "Choose model".into(),
            |model| model.display_name.clone().into(),
        );
        let effort_label: SharedString = selected_effort.as_ref().map_or_else(
            || "Effort".into(),
            |effort| effort.label().to_owned().into(),
        );
        let selected_provider = selected_model.as_ref().map_or_else(
            || provider_icon(None),
            |model| provider_icon(Some(&model.provider)),
        );
        let trigger = Button::new("prompt-model")
            .ghost()
            .accessibility_id("prompt-model-and-effort-selector")
            .debug_selector(|| "prompt-model-selector".into())
            .h(px(32.))
            .max_w(px(260.))
            .px(px(10.))
            .gap(px(7.))
            .rounded(px(9.))
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.08))
            .bg(super::super::super::visual::surface(
                super::super::super::visual::SurfaceLevel::Raised,
                cx,
            ))
            .child(selected_provider)
            .child(
                div()
                    .min_w_0()
                    .text_size(px(12.))
                    .font_medium()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(model_label),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(effort_label),
            )
            .child(
                Icon::new(IconName::ChevronDown)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            );

        Popover::new("prompt-model-picker")
            .anchor(Anchor::BottomRight)
            .trigger(trigger)
            .appearance(false)
            .content(move |_, window, popover_cx| {
                Self::model_picker_surface(
                    &models,
                    selected_model.as_ref(),
                    automatic_selected,
                    model_unavailable,
                    &efforts,
                    selected_effort.as_ref(),
                    effort_unavailable,
                    &view,
                    window,
                    popover_cx,
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn model_picker_surface(
        models: &[ModelDescriptor],
        selected_model: Option<&ModelDescriptor>,
        automatic_selected: bool,
        model_unavailable: bool,
        efforts: &[EffortLevel],
        selected_effort: Option<&EffortLevel>,
        effort_unavailable: bool,
        view: &Entity<Self>,
        window: &Window,
        cx: &Context<'_, PopoverState>,
    ) -> AnyElement {
        let popover = cx.entity();
        let unavailable_model = selected_model
            .filter(|selected| {
                model_unavailable
                    && !models
                        .iter()
                        .any(|model| model.provider == selected.provider && model.id == selected.id)
            })
            .cloned();
        let model_rows = models.iter().map(|model| {
            Self::model_picker_model_row(
                model,
                selected_model.filter(|_| !automatic_selected),
                true,
                view,
                window,
                cx,
            )
        });
        let effort_rows = efforts.iter().map(|effort| {
            Self::model_picker_effort_row(
                effort,
                selected_effort,
                effort_unavailable,
                view,
                &popover,
                cx,
            )
        });

        h_flex()
            .debug_selector(|| "prompt-model-picker-surface".into())
            .w(px(424.))
            .max_h(px(420.))
            .items_stretch()
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().foreground.opacity(0.09))
            .bg(super::super::super::visual::surface(
                super::super::super::visual::SurfaceLevel::Floating,
                cx,
            ))
            .shadow(super::super::super::visual::floating_shadow(cx))
            .child(
                v_flex()
                    .w(px(256.))
                    .min_h(px(150.))
                    .p(px(8.))
                    .gap(px(3.))
                    .child(picker_heading("Models", cx))
                    .child(Self::automatic_model_row(automatic_selected, view, cx))
                    .when_some(unavailable_model.as_ref(), |this, model| {
                        this.child(Self::model_picker_model_row(
                            model,
                            selected_model,
                            false,
                            view,
                            window,
                            cx,
                        ))
                    })
                    .children(model_rows),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(150.))
                    .border_l_1()
                    .border_color(cx.theme().foreground.opacity(0.07))
                    .p(px(8.))
                    .gap(px(3.))
                    .child(picker_heading("Effort", cx))
                    .children(effort_rows),
            )
            .into_any_element()
    }

    fn model_picker_model_row(
        model: &ModelDescriptor,
        selected_model: Option<&ModelDescriptor>,
        selectable: bool,
        view: &Entity<Self>,
        window: &Window,
        cx: &App,
    ) -> Button {
        let selected = selected_model
            .is_some_and(|selected| selected.provider == model.provider && selected.id == model.id);
        let model_for_click = model.clone();

        Button::new(SharedString::from(format!(
            "model-{}-{}",
            model.provider.0, model.id.0
        )))
        .ghost()
        .w_full()
        .h(px(44.))
        .px(px(9.))
        .rounded(px(9.))
        .border_1()
        .border_color(if selected {
            cx.theme().primary.opacity(0.28)
        } else {
            cx.theme().foreground.opacity(0.)
        })
        .bg(if selected {
            cx.theme().accent.opacity(0.72)
        } else {
            cx.theme().accent.opacity(0.)
        })
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap(px(9.))
                .child(provider_icon(Some(&model.provider)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .font_medium()
                                .child(model.display_name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground)
                                .child(provider_menu_label(&model.provider)),
                        ),
                )
                .when(selected, |this| {
                    this.child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                }),
        )
        .when(selectable, |this| {
            this.on_click(window.listener_for(view, move |composer, _, _, cx| {
                composer.select_model(model_for_click.clone(), cx);
            }))
        })
    }

    fn automatic_model_row(selected: bool, view: &Entity<Self>, cx: &App) -> Button {
        let automatic_view = view.clone();
        Button::new("model-automatic")
            .ghost()
            .w_full()
            .h(px(44.))
            .px(px(9.))
            .rounded(px(9.))
            .border_1()
            .border_color(if selected {
                cx.theme().primary.opacity(0.28)
            } else {
                cx.theme().foreground.opacity(0.)
            })
            .bg(if selected {
                cx.theme().accent.opacity(0.72)
            } else {
                cx.theme().accent.opacity(0.)
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(9.))
                    .child(provider_icon(None))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(div().text_size(px(12.)).font_medium().child("Automatic"))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Use the current default"),
                            ),
                    )
                    .when(selected, |this| {
                        this.child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                    }),
            )
            .on_click(move |_, _, cx| {
                automatic_view.update(cx, Self::select_automatic);
            })
    }

    fn model_picker_effort_row(
        effort: &EffortLevel,
        selected_effort: Option<&EffortLevel>,
        effort_unavailable: bool,
        view: &Entity<Self>,
        popover: &Entity<PopoverState>,
        cx: &App,
    ) -> Button {
        let selected = selected_effort == Some(effort);
        let effort_for_click = effort.clone();
        let select_view = view.clone();
        let dismiss = popover.clone();

        Button::new(SharedString::from(format!(
            "effort-{}",
            effort.wire_value()
        )))
        .ghost()
        .w_full()
        .h(px(44.))
        .px(px(9.))
        .rounded(px(9.))
        .border_1()
        .border_color(if selected {
            cx.theme().primary.opacity(0.28)
        } else {
            cx.theme().foreground.opacity(0.)
        })
        .bg(if selected {
            cx.theme().accent.opacity(0.72)
        } else {
            cx.theme().accent.opacity(0.)
        })
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .items_start()
                        .gap(px(2.))
                        .child(div().text_size(px(12.)).font_medium().child(
                            if effort_unavailable {
                                format!("{} (unavailable)", effort.label())
                            } else {
                                effort.label().to_owned()
                            },
                        ))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground)
                                .child(effort_description(effort)),
                        ),
                )
                .when(selected, |this| {
                    this.child(div().size(px(5.)).rounded_full().bg(cx.theme().primary))
                }),
        )
        .on_click(move |_, window, cx| {
            select_view.update(cx, |composer, cx| {
                composer.select_effort(effort_for_click.clone(), cx);
            });
            dismiss.update(cx, |state, cx| state.dismiss(window, cx));
        })
    }
}
