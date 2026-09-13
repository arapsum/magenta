use magenta_core::ProviderAccount;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileDetails {
    pub(super) name: String,
    pub(super) detail: String,
    pub(super) initial: String,
    pub(super) tooltip: String,
    pub(super) email: Option<String>,
    pub(super) plan: Option<String>,
}

pub fn profile_details(account: Option<&ProviderAccount>) -> ProfileDetails {
    let Some(account) = account else {
        return ProfileDetails {
            name: "Adleio".to_owned(),
            detail: "Local profile".to_owned(),
            initial: "A".to_owned(),
            tooltip: "Local profile and settings".to_owned(),
            email: None,
            plan: None,
        };
    };

    let name = account
        .name
        .clone()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| account.email.as_deref().and_then(email_display_name))
        .or_else(|| account.email.clone())
        .unwrap_or_else(|| "ChatGPT account".to_owned());
    let email = account.email.clone();
    let plan = account.plan.as_deref().and_then(display_plan);
    let detail = match (plan.as_deref(), email.as_deref()) {
        (Some(plan), Some(email)) => format!("{plan} · {email}"),
        (Some(plan), None) => plan.to_owned(),
        (None, Some(email)) => email.to_owned(),
        (None, None) => "OpenAI account".to_owned(),
    };
    let initial = name.chars().next().map_or_else(
        || "O".to_owned(),
        |character| character.to_uppercase().collect(),
    );
    let tooltip = account.email.clone().map_or_else(
        || "OpenAI account and settings".to_owned(),
        |email| format!("{name} · {email}"),
    );

    ProfileDetails {
        name,
        detail,
        initial,
        tooltip,
        email,
        plan,
    }
}

fn display_plan(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| capitalize(value))
}

fn email_display_name(email: &str) -> Option<String> {
    let local_part = email.split('@').next()?.trim();
    let parts = local_part
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(capitalize)
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };

    let mut capitalized = first.to_uppercase().collect::<String>();
    capitalized.push_str(characters.as_str());
    capitalized
}
