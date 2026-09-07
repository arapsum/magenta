use gpui::{
    App, Context, InteractiveElement as _, IntoElement, Modifiers, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, div, px,
};
use gpui_component::{ActiveTheme as _, button::Button};
use magenta_core::{ProviderAccount, ProviderId};

use super::{ProfileDetails, account_dropdown::AccountDropdown, profile_details};

struct AccountDropdownHarness;

impl Render for AccountDropdownHarness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_end()
            .p(px(20.))
            .child(AccountDropdown::new(
                "account-dropdown-test",
                Button::new("account-dropdown-test-trigger")
                    .w(px(200.))
                    .h(px(40.))
                    .label("Account"),
                |_, _, cx: &mut App| {
                    div()
                        .debug_selector(|| "account-dropdown-test-menu".into())
                        .w(px(180.))
                        .h(px(100.))
                        .bg(cx.theme().popover)
                        .into_any_element()
                },
            ))
    }
}

#[gpui::test]
fn account_dropdown_opens_and_dismisses_from_an_outside_click(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|_, _| AccountDropdownHarness);
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let trigger = cx.debug_bounds("account-dropdown-trigger").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.debug_bounds("account-dropdown-test-menu").is_some());

    cx.simulate_click(gpui::point(px(5.), px(5.)), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.debug_bounds("account-dropdown-test-menu").is_none());
}

#[test]
fn signed_out_profile_keeps_the_local_fallback() {
    assert_eq!(
        profile_details(None),
        ProfileDetails {
            name: "Adleio".to_owned(),
            detail: "Local profile".to_owned(),
            initial: "A".to_owned(),
            tooltip: "Local profile and settings".to_owned(),
            email: None,
            plan: None,
        }
    );
}

#[test]
fn connected_profile_uses_name_email_and_initial() {
    let account = ProviderAccount {
        provider: ProviderId::new("openai"),
        name: Some("Jacob Cooper".to_owned()),
        email: Some("jacob@example.com".to_owned()),
        plan: Some("plus".to_owned()),
    };

    assert_eq!(
        profile_details(Some(&account)),
        ProfileDetails {
            name: "Jacob Cooper".to_owned(),
            detail: "Plus · jacob@example.com".to_owned(),
            initial: "J".to_owned(),
            tooltip: "Jacob Cooper · jacob@example.com".to_owned(),
            email: Some("jacob@example.com".to_owned()),
            plan: Some("Plus".to_owned()),
        }
    );
}

#[test]
fn connected_profile_derives_a_friendly_name_when_claims_have_no_name() {
    let account = ProviderAccount {
        provider: ProviderId::new("openai"),
        name: None,
        email: Some("jacob.cooper@example.com".to_owned()),
        plan: Some("plus".to_owned()),
    };

    let details = profile_details(Some(&account));

    assert_eq!(details.name, "Jacob Cooper");
    assert_eq!(details.detail, "Plus · jacob.cooper@example.com");
    assert_eq!(details.initial, "J");
    assert_eq!(details.email.as_deref(), Some("jacob.cooper@example.com"));
    assert_eq!(details.plan.as_deref(), Some("Plus"));
}
