use super::*;

#[test]
fn catalog_requests_a_client_version_that_exposes_gpt_6_1() {
    let client = http_client::FakeHttpClient::create(|request| async move {
        let url = Url::parse(&request.uri().to_string()).unwrap();
        assert_eq!(url.path(), "/backend-api/codex/models");
        let version = url
            .query_pairs()
            .find_map(|(key, value)| (key == "client_version").then(|| value.into_owned()))
            .expect("catalog discovery must advertise Codex compatibility");
        assert_eq!(request.headers()["version"], version);
        assert_eq!(request.headers()["authorization"], "Bearer test-token");
        let version = version
            .split('.')
            .map(|part| part.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        // GPT-6.1 Terra is gated on Codex 0.159.0; older clients see the old catalog.
        let models = if version.as_slice() >= [0, 159, 0].as_slice() {
            serde_json::json!({"models": [
                {"slug": "gpt-6.1-sol", "display_name": "GPT-6.1 Sol", "visibility": "list"},
                {"slug": "gpt-6.1-terra", "display_name": "GPT-6.1 Terra", "visibility": "list"},
                {"slug": "gpt-6-sol", "display_name": "GPT-6 Sol", "visibility": "list"},
                {"slug": "gpt-6-luna", "display_name": "GPT-6 Luna", "visibility": "list"}
            ]})
        } else {
            serde_json::json!({"models": [{"slug": "gpt-5.6-sol", "visibility": "list"}]})
        };
        Ok(http_client::Response::builder()
            .status(200)
            .body(http_client::AsyncBody::from(models.to_string()))
            .unwrap())
    });
    let provider = OpenAiProvider::with_client(client);
    let mut response = smol::block_on(provider.send_models("test-token")).unwrap();
    let body = smol::block_on(http::read_limited(response.body_mut(), MAX_MODEL_BODY)).unwrap();
    let models = model_descriptors(serde_json::from_slice::<ModelsResponse>(&body).unwrap());
    let ids = models
        .iter()
        .map(|model| model.id.0.as_str())
        .collect::<Vec<_>>();
    for id in ["gpt-6.1-sol", "gpt-6.1-terra", "gpt-6-sol", "gpt-6-luna"] {
        assert!(
            ids.contains(&id),
            "expected the current catalog to include {id}"
        );
    }
}
