//! Fixed, credential-safe RPC authentication diagnostics. Never echo server text.
//! RPC codes: https://docs.discord.com/developers/topics/opcodes-and-status-codes#rpc
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    Scope,
    Client,
    Grant,
    Redirect,
    Denied,
}
impl Reason {
    fn from_text(text: &str) -> Option<Self> {
        if text.len() > 2048 {
            return None;
        }
        // Only complete OAuth identifiers count. Never copy the surrounding
        // message, which may contain tokens, credentials, URLs or user data.
        text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .find_map(|tag| match tag {
                "invalid_scope" => Some(Self::Scope),
                "invalid_client" => Some(Self::Client),
                "invalid_grant" => Some(Self::Grant),
                "redirect_uri" | "redirect_uri_mismatch" | "invalid_redirect_uri" => {
                    Some(Self::Redirect)
                }
                "access_denied" => Some(Self::Denied),
                _ => None,
            })
    }
    fn guidance(self) -> &'static str {
        match self {
            Self::Scope => {
                "OAuth2 invalid_scope. Check the requested permissions against this application's allowed scopes, then Connect again."
            }
            Self::Client => {
                "OAuth2 invalid_client. Check that the Application ID and client secret belong to the same Discord application."
            }
            Self::Grant => {
                "OAuth2 invalid_grant. The authorization code or token was rejected. Use Connect to authorize again."
            }
            Self::Redirect => {
                "OAuth2 redirect URI error. Check that the configured redirect URI exactly matches an allowed URI for this Discord application."
            }
            Self::Denied => {
                "OAuth2 access_denied. Authorization was denied. Use Connect when ready to grant access."
            }
        }
    }
}
fn reason(value: &Value, depth: usize, budget: &mut usize) -> Option<Reason> {
    if depth > 4 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    match value {
        Value::String(text) => Reason::from_text(text),
        Value::Object(object) => {
            // Explicit fields only, avoiding traversal of unrelated user data.
            for key in ["error", "data", "message", "error_description", "errors"] {
                if let Some(found) = object.get(key).and_then(|v| reason(v, depth + 1, budget)) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(values) => values
            .iter()
            .take(8)
            .find_map(|v| reason(v, depth + 1, budget)),
        _ => None,
    }
}

pub fn describe(stage: &str, data: &Value) -> String {
    let stage = match stage {
        "AUTHORIZE" => "AUTHORIZE",
        "AUTHENTICATE" => "AUTHENTICATE",
        _ => "authentication",
    };
    let code = data.get("code").and_then(Value::as_i64);
    let detail = match code {
        Some(4007) => "Invalid OAuth2 Application ID. Check the Application ID in the plugin's Connect settings.",
        Some(4008) => "Invalid OAuth2 origin. Check this application's allowed RPC origins and client configuration.",
        Some(4009) => "Invalid OAuth2 token. Use Connect to authorize again.",
        Some(5000) => reason(data, 0, &mut 32).map(Reason::guidance).unwrap_or("OAuth2 error; the specific reason is unknown. Discord did not provide a recognized safe error identifier."),
        _ => "The authentication failure reason is unknown; no recognized RPC authentication error code was provided.",
    };
    let code = code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "unknown".into());
    format!("Discord {stage} failed (RPC code {code}). {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documented_rpc_codes_have_distinct_stage_specific_guidance() {
        for (code, expected) in [(4007, "Application ID"), (4008, "origin"), (4009, "token")] {
            let text = describe(
                "AUTHENTICATE",
                &json!({"code":code,"message":"untrusted-secret"}),
            );
            assert!(text.contains("AUTHENTICATE"));
            assert!(text.contains(&code.to_string()));
            assert!(text.contains(expected));
            assert!(!text.contains("untrusted-secret"));
        }
    }
    #[test]
    fn known_nested_oauth_tags_are_actionable_without_echoing_server_data() {
        for (tag, expected) in [
            ("invalid_scope", "allowed scopes"),
            ("invalid_client", "same Discord application"),
            ("invalid_grant", "authorize again"),
            ("redirect_uri_mismatch", "exactly matches"),
            ("access_denied", "was denied"),
        ] {
            for data in [
                json!({"code":5000,"error":tag}),
                json!({"code":5000,"data":{"error":{"message":format!("{tag}: NEVER_ECHO_SECRET")}}}),
                json!({"code":5000,"errors":[{"message":tag}]}),
            ] {
                let text = describe("AUTHORIZE", &data);
                assert!(text.contains("AUTHORIZE failed (RPC code 5000)"));
                assert!(text.contains(expected), "{text}");
                assert!(!text.contains("NEVER_ECHO_SECRET"));
            }
        }
    }
    #[test]
    fn unknown_malicious_or_malformed_details_do_not_leak_or_assert_approval() {
        for data in [
            json!({"code":5000,"message":"client_secret=NEVER_ECHO_SECRET; https://private.example/token"}),
            json!({"code":5000,"error":"prefixinvalid_scope_suffix"}),
            json!({"code":"NEVER_ECHO_SECRET","message":"invalid_client"}),
            json!({"code":5000,"credentials":{"error":"invalid_scope"}}),
            Value::Null,
        ] {
            let text = describe("NEVER_ECHO_SECRET", &data);
            assert!(text.contains("unknown"));
            assert!(!text.contains("NEVER_ECHO_SECRET"));
            assert!(!text.contains("private.example"));
            assert!(!text.contains("approval"));
        }
    }
    #[test]
    fn observed_missing_redirect_error_identifies_redirect_configuration() {
        let text = describe(
            "AUTHORIZE",
            &json!({
                "code": 5000,
                "message": "OAuth2 Error: invalid_request: Missing \"redirect_uri\" in request."
            }),
        );
        assert!(text.contains("AUTHORIZE failed (RPC code 5000)"));
        assert!(text.contains("redirect URI"));
        assert!(text.contains("allowed URI for this Discord application"));
        assert!(!text.contains("unknown"));
        assert!(!text.contains("Missing \"redirect_uri\" in request"));
    }

    #[test]
    fn parsing_is_bounded_and_repeatable() {
        let secret =
            json!({"code":5000,"error":{"data":{"error":{"data":{"error":"invalid_scope"}}}}});
        assert!(describe("AUTHORIZE", &secret).contains("unknown"));
        let large = json!({"code":5000,"message":format!("invalid_scope {}", "s".repeat(4096))});
        assert!(describe("AUTHORIZE", &large).contains("unknown"));
        let sample = json!({"code":5000,"data":{"error":"invalid_scope","message":"secret"}});
        let expected = describe("AUTHORIZE", &sample);
        for _ in 0..100 {
            assert_eq!(describe("AUTHORIZE", &sample), expected);
        }
    }
}
