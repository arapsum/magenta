use futures_util::StreamExt as _;
use magenta_core::{
    ChatProvider, GenerationEvent, GenerationOutcome, GenerationRequest, ProviderError,
};

pub(crate) fn assert_success_contract(
    provider: &dyn ChatProvider,
    request: GenerationRequest,
    expected_text: &str,
) -> GenerationOutcome {
    let events = smol::block_on(provider.stream(request).collect::<Vec<_>>());
    assert!(matches!(events.first(), Some(Ok(GenerationEvent::Started))));

    let mut started = 0;
    let mut response = String::new();
    let mut outcome = None;
    for (index, event) in events.iter().enumerate() {
        match event
            .as_ref()
            .expect("a successful provider stream must not emit an error")
        {
            GenerationEvent::Started => started += 1,
            GenerationEvent::TextDelta(chunk) => {
                assert!(outcome.is_none(), "text cannot follow completion");
                response.push_str(chunk);
            }
            GenerationEvent::Completed(completed) => {
                assert_eq!(index + 1, events.len(), "completion must be final");
                assert!(outcome.replace(completed.clone()).is_none());
            }
        }
    }

    assert_eq!(started, 1, "a successful stream must start exactly once");
    assert_eq!(response, expected_text);
    outcome.expect("a successful stream must complete exactly once")
}

pub(crate) fn assert_failure_contract(
    provider: &dyn ChatProvider,
    request: GenerationRequest,
) -> ProviderError {
    let mut events = smol::block_on(provider.stream(request).collect::<Vec<_>>());
    assert_eq!(
        events.iter().filter(|event| event.is_err()).count(),
        1,
        "a failed stream must emit one error"
    );
    assert!(
        events.last().is_some_and(Result::is_err),
        "the provider error must be terminal"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, Ok(GenerationEvent::Completed(_)))),
        "a failed stream must not complete"
    );
    let started = events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event, Ok(GenerationEvent::Started)))
        .collect::<Vec<_>>();
    assert!(started.len() <= 1, "a failed stream may start at most once");
    assert!(
        started.first().is_none_or(|(index, _)| *index == 0),
        "a started event must be first"
    );

    events
        .pop()
        .expect("a failed stream must emit an event")
        .expect_err("the provider error must be terminal")
}
