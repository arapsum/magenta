#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    NewChat,
    Models,
    ImageLibrary,
    Experts,
    Collaborate,
    TrustScore,
    Billing,
    SearchChat,
    AddFolder,
    Project,
    Recent,
    StartupStrategy,
    SocialContent,
}

impl Destination {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::NewChat => "new-chat",
            Self::Models => "models",
            Self::ImageLibrary => "image-library",
            Self::Experts => "experts",
            Self::Collaborate => "collaborate",
            Self::TrustScore => "trust-score",
            Self::Billing => "billing",
            Self::SearchChat => "search-chat",
            Self::AddFolder => "add-folder",
            Self::Project => "project",
            Self::Recent => "recent",
            Self::StartupStrategy => "startup-strategy",
            Self::SocialContent => "social-content",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NewChat => "New Chat",
            Self::Models => "Models",
            Self::ImageLibrary => "Image Library",
            Self::Experts => "Experts",
            Self::Collaborate => "Collaborate",
            Self::TrustScore => "Trust Score",
            Self::Billing => "Billing",
            Self::SearchChat => "Search Chat",
            Self::AddFolder => "Add New Folder",
            Self::Project => "New project",
            Self::Recent => "Recent Conversations",
            Self::StartupStrategy => "Startup marketing strategy",
            Self::SocialContent => "Content ideas for social media",
        }
    }
}
