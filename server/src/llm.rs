//! OpenAI-compatible inference transport. Upstream providers belong behind the server.

use std::fmt;

pub enum Provider {
    Stub,
    OpenAiCompatible {
        model: String,
        base: String,
        api_key: Option<String>,
        client: reqwest::Client,
        max_tokens: u32,
    },
}

impl fmt::Debug for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stub => write!(f, "Stub"),
            Self::OpenAiCompatible { model, .. } => write!(f, "OpenAiCompatible({model})"),
        }
    }
}

impl Provider {
    /// Only reads connection settings for the user's inference server.
    /// Missing settings select the stub; partial configuration is an error.
    pub fn from_env() -> Result<Self, String> {
        Self::from_settings(|name| std::env::var(name).ok())
    }

    fn from_settings(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let setting = |name| get(name).filter(|s| !s.trim().is_empty());
        let base = setting("LLM_BASE_URL");
        let model = setting("LLM_MODEL");
        let api_key = setting("LLM_API_KEY");
        let max_tokens = setting("LLM_MAX_TOKENS");
        if base.is_none() && model.is_none() && api_key.is_none() && max_tokens.is_none() {
            return Ok(Self::Stub);
        }
        let base = base.ok_or("LLM_BASE_URL is required when configuring an inference server")?;
        let model = model.ok_or("LLM_MODEL is required when configuring an inference server")?;
        let max_tokens = max_tokens
            .map(|s| s.trim().parse::<u32>())
            .transpose()
            .map_err(|_| "LLM_MAX_TOKENS must be a positive integer")?
            .unwrap_or(1024);
        Self::openai_compatible(&base, &model, api_key, max_tokens)
    }

    /// `base` includes the server's API prefix (usually `/v1`).
    /// `api_key` authenticates to that server, and may be omitted for local servers.
    pub fn openai_compatible(
        base: &str,
        model: &str,
        api_key: Option<String>,
        max_tokens: u32,
    ) -> Result<Self, String> {
        let base = base.trim().trim_end_matches('/');
        let url = reqwest::Url::parse(base).map_err(|_| "LLM_BASE_URL must be an HTTP(S) URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "LLM_BASE_URL must be an HTTP(S) URL without credentials, query, or fragment"
                    .into(),
            );
        }
        if model.trim().is_empty() {
            return Err("LLM_MODEL must not be empty".into());
        }
        if max_tokens == 0 {
            return Err("LLM_MAX_TOKENS must be a positive integer".into());
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|_| "could not create the inference HTTP client")?;
        Ok(Self::OpenAiCompatible {
            model: model.trim().into(),
            base: base.into(),
            api_key: api_key.filter(|s| !s.trim().is_empty()),
            client,
            max_tokens,
        })
    }

    pub(crate) async fn complete(
        &self,
        system: &str,
        user: &str,
        temperature: f32,
    ) -> Option<String> {
        let Self::OpenAiCompatible {
            model,
            base,
            api_key,
            client,
            max_tokens,
        } = self
        else {
            return None;
        };
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": false,
        });
        let mut request = client.post(format!("{base}/chat/completions")).json(&body);
        if let Some(key) = api_key {
            request = request.bearer_auth(key);
        }
        let started = std::time::Instant::now();
        // Do not log request URLs, headers, bodies, or raw errors: they can contain secrets.
        let Ok(resp) = request.send().await else {
            tracing::warn!(
                ms = started.elapsed().as_millis() as u64,
                "inference request failed"
            );
            return None;
        };
        if !resp.status().is_success() {
            tracing::warn!(
                status = resp.status().as_u16(),
                ms = started.elapsed().as_millis() as u64,
                "inference request failed"
            );
            return None;
        }
        let Ok(payload) = resp.json::<serde_json::Value>().await else {
            tracing::warn!("inference response was not json");
            return None;
        };
        let text = payload
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            tracing::warn!("inference response had no text");
            return None;
        }
        tracing::info!(ms = started.elapsed().as_millis() as u64, "inference reply");
        Some(text.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(settings: &[(&str, &str)]) -> Result<Provider, String> {
        Provider::from_settings(|name| {
            settings
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    #[test]
    fn configuration_only_uses_inference_server_settings() {
        // Upstream credentials alone must never enable network access.
        let provider =
            configured(&[("GEMINI_API_KEY", "secret"), ("NVIDIA_API_KEY", "secret")]).unwrap();
        assert!(matches!(provider, Provider::Stub));
        assert!(matches!(
            configured(&[("LLM_BASE_URL", "  ")]).unwrap(),
            Provider::Stub
        ));
        for settings in [
            vec![("LLM_MODEL", "town")],
            vec![("LLM_BASE_URL", "http://localhost:4000/v1")],
            vec![("LLM_API_KEY", "secret")],
            vec![("LLM_BASE_URL", "file:///tmp/llm"), ("LLM_MODEL", "town")],
            vec![
                ("LLM_BASE_URL", "https://user:secret@example.com/v1"),
                ("LLM_MODEL", "town"),
            ],
            vec![
                ("LLM_BASE_URL", "http://localhost/v1?key=secret"),
                ("LLM_MODEL", "town"),
            ],
        ] {
            let error = configured(&settings).unwrap_err();
            assert!(!error.contains("secret"));
        }
        for value in ["0", "-1", "invalid"] {
            assert!(
                configured(&[
                    ("LLM_BASE_URL", "http://localhost/v1"),
                    ("LLM_MODEL", "town"),
                    ("LLM_MAX_TOKENS", value)
                ])
                .is_err()
            );
        }
        let provider = configured(&[
            ("LLM_BASE_URL", " http://localhost:4000/v1/ "),
            ("LLM_MODEL", " town "),
        ])
        .unwrap();
        let Provider::OpenAiCompatible {
            base,
            model,
            api_key,
            max_tokens,
            ..
        } = provider
        else {
            panic!("expected inference server")
        };
        assert_eq!(base, "http://localhost:4000/v1");
        assert_eq!(model, "town");
        assert!(api_key.is_none());
        assert_eq!(max_tokens, 1024);
    }

    #[tokio::test]
    async fn sends_the_chat_contract_with_optional_server_authentication() {
        for api_key in [None, Some("proxy-token".to_string())] {
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            let app = axum::Router::new().route("/proxy/v1/chat/completions", axum::routing::post(
                move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<serde_json::Value>| async move {
                    tx.send((headers, body)).await.unwrap();
                    axum::Json(serde_json::json!({"choices": [{"message": {"content": "  Hello, town!  "}}]}))
                }
            ));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/proxy/v1/", listener.local_addr().unwrap());
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let provider =
                Provider::openai_compatible(&base, "town-model", api_key.clone(), 512).unwrap();
            assert_eq!(
                provider
                    .complete("You are Lucky.", "Say hello.", 0.5)
                    .await
                    .as_deref(),
                Some("Hello, town!")
            );
            let (headers, body) = rx.recv().await.unwrap();
            assert_eq!(
                headers
                    .get("authorization")
                    .map(|h| h.to_str().unwrap().to_string()),
                api_key.map(|key| format!("Bearer {key}"))
            );
            assert_eq!(
                body,
                serde_json::json!({
                    "model": "town-model",
                    "messages": [{"role": "system", "content": "You are Lucky."}, {"role": "user", "content": "Say hello."}],
                    "max_tokens": 512, "temperature": 0.5, "stream": false,
                })
            );
            task.abort();
        }
    }
}
