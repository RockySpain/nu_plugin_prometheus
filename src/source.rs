use nu_json::{Map, Value};
use nu_plugin::EvaluatedCall;
use nu_protocol::{record, LabeledError, Span};
use prometheus_http_query::Client;
use reqwest::{Certificate, Identity};
use std::path::Path;

pub struct Source {
    pub name: Option<String>,
    pub url: String,
    pub identity: Option<Identity>,
    pub cacert: Option<Certificate>,
    pub span: Span,
}

impl Source {
    pub fn list() -> Result<nu_protocol::Value, LabeledError> {
        let span = Span::unknown();
        let sources = load_sources()?;

        let mut list = vec![];
        for (name, source) in sources.iter() {
            let Value::Object(source) = source else {
                return Err(LabeledError::new(
                    format!("source {name:?} in configuration is not an object"))
                );
            };

            let entry = nu_protocol::Value::record(
                record!(
                    "name" => nu_protocol::Value::string(name, span.clone()),
                    "url" => get_field_value(&source, "url"),
                    "cert" => get_field_value(&source, "cert"),
                    "key" => get_field_value(&source, "key"),
                    "cacert" => get_field_value(&source, "cacert"),
                ),
                span.clone(),
            );

            list.push(entry);
        }

        Ok(nu_protocol::Value::list(list, span.clone()))
    }

    pub fn from_call(call: &EvaluatedCall) -> Result<Self, LabeledError> {
        let source = call.get_flag_value("source");
        let url = call.get_flag_value("url");

        if let Some(url) = url {
            if let Some(source) = source {
                return Err(LabeledError::new("Argument error")
                    .with_label("Supply only --source or --url, not both", source.span()));
            }

            Source::from_call_url(call, url)
        } else if let Some(source) = source {
            Source::from_config(source)
        } else {
            Err(LabeledError::new("Argument error")
                .with_label("Missing --source or --url flag", call.head))
        }
    }

    fn from_call_url(
        call: &EvaluatedCall,
        url_value: nu_protocol::Value,
    ) -> Result<Self, LabeledError> {
        let nu_protocol::Value::String { val: ref url, .. } = url_value else {
            return Err(LabeledError::new("Invalid argument type")
                .with_label("Expected --url to be a String", url_value.span()));
        };

        let cert = call.get_flag_value("cert");
        let key = call.get_flag_value("key");

        let identity = make_identity(cert, key)?;

        let cacert = call
            .get_flag_value("cacert")
            .map(|cacert| certificate(cacert))
            .transpose()?;

        Ok(Self {
            name: None,
            url: url.clone(),
            identity,
            cacert,
            span: url_value.span(),
        })
    }

    fn from_config(source: nu_protocol::Value) -> Result<Self, LabeledError> {
        let name = source.clone().into_string().unwrap();

        let sources = load_sources()?;

        let Some(chosen) = sources.get(&name) else {
            return Err(LabeledError::new("Unable to find source").with_label(
                format!("source {name:?} in configuration does not exist"),
                source.span()));
        };

        let Value::Object(chosen) = chosen else {
            return Err(LabeledError::new("Unable to find source").with_label(
                format!("source {name:?} in configuration is not an object"),
                source.span()));
        };

        let url = get_field(&chosen, "url");

        let Some(url) = url else {
                return Err(LabeledError::new("Unable to find source url").with_label(
                    format!("source {name:?} in configuration is missing its \"url\" field"),
                    source.span()));
            };

        let cert = get_field(&chosen, "cert")
            .map(|cert| nu_protocol::Value::string(cert, Span::unknown()));
        let key =
            get_field(&chosen, "key").map(|key| nu_protocol::Value::string(key, Span::unknown()));

        let identity = make_identity(cert, key)?;

        let cacert = get_field(&chosen, "cacert")
            .map(|cacert| nu_protocol::Value::string(cacert, Span::unknown()))
            .map(|cacert| certificate(cacert))
            .transpose()?;

        let chosen = Self {
            name: Some(name),
            url,
            identity,
            cacert,
            span: source.span(),
        };

        Ok(chosen)
    }
}

impl TryFrom<&EvaluatedCall> for Source {
    type Error = LabeledError;

    fn try_from(call: &EvaluatedCall) -> Result<Self, Self::Error> {
        Source::from_call(call)
    }
}

impl TryFrom<Source> for Client {
    type Error = LabeledError;

    fn try_from(source: Source) -> Result<Self, Self::Error> {
        let client_builder = reqwest::ClientBuilder::new();

        let client_builder = if let Some(identity) = source.identity {
            client_builder.identity(identity)
        } else {
            client_builder
        };

        let client_builder = if let Some(cacert) = source.cacert {
            client_builder.add_root_certificate(cacert)
        } else {
            client_builder
        };

        let client = client_builder.build().map_err(|e| {
            LabeledError::new("Unable to build prometheus client").with_help(e.to_string())
        })?;

        let client = Client::from(client, &source.url).map_err(|e| {
            LabeledError::new("Unable to build prometheus client").with_help(e.to_string())
        })?;

        Ok(client)
    }
}

fn certificate(cacert: nu_protocol::Value) -> Result<Certificate, LabeledError> {
    let cacert_pem = read_pem(&cacert, "CA certificate")?;

    let cacert = Certificate::from_pem(&cacert_pem).map_err(|e| {
        LabeledError::new(e.to_string()).with_label(
            format!(
                "CA certificate {} is not in PEM format",
                cacert.clone().into_string().unwrap()
            ),
            cacert.span(),
        )
    })?;

    Ok(cacert)
}

fn get_field(chosen: &Map<String, Value>, field: &str) -> Option<String> {
    chosen
        .get(field)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn get_field_value(chosen: &Map<String, Value>, field: &str) -> nu_protocol::Value {
    let span = Span::unknown();

    get_field(chosen, field).map_or(nu_protocol::Value::nothing(span), |url| {
        nu_protocol::Value::string(url, span)
    })
}

fn identity(cert: nu_protocol::Value, key: nu_protocol::Value) -> Result<Identity, LabeledError> {
    let cert_pem = read_pem(&cert, "Client certificate")?;
    let key_pem = read_pem(&key, "Client key")?;

    Identity::from_pkcs8_pem(&cert_pem, &key_pem).map_err(|e| {
        LabeledError::new("Client certificate or key are not in PEM format")
            .with_help(e.to_string())
    })
}

fn load_sources() -> Result<Map<String, nu_json::Value>, LabeledError> {
    let home = std::env::var("HOME").map_err(|e| {
        LabeledError::new("Unable to load sources")
            .with_help(format!("Could not find HOME env var: {e:?}"))
    })?;

    let config_file = Path::new(&home).join(".config/nu_plugin_prometheus.hjson");

    let config: Map<String, Value> = std::fs::read(&config_file)
        .map(|config| nu_json::from_slice(&config[..]))
        .map_err(|e| {
            LabeledError::new("Unable to load sources").with_help(format!(
                "Unable to read configuration file at {config_file:?}: {e:?}"
            ))
        })?
        .map_err(|e| {
            LabeledError::new("Unable to load sources").with_help(format!(
                "Unable to deserialize configuration file at {config_file:?}: {e:?}"
            ))
        })?;

    let Some(sources) = config.get("sources") else {
        return Err(LabeledError::new("Unable to load sources").with_help(
            format!("Configuration file at {config_file:?} is missing a \"sources\" entry")));
    };

    let Value::Object(sources) = sources else {
        return Err(LabeledError::new( "Unable to load sources").with_help(
            format!("\"sources\" entry in configuration file at {config_file:?} is not a object")));
    };

    Ok(sources.clone())
}

fn make_identity(
    cert: Option<nu_protocol::Value>,
    key: Option<nu_protocol::Value>,
) -> Result<Option<Identity>, LabeledError> {
    match (cert, key) {
        (None, None) => Ok(None),
        (Some(cert), Some(key)) => Ok(Some(identity(cert, key)?)),
        (None, Some(key)) => {
            return Err(missing_flag("client key", "--cert", key.span()));
        }
        (Some(cert), None) => {
            return Err(missing_flag("client cert", "--key", cert.span()));
        }
    }
}

fn missing_flag(have: &str, missing: &str, span: Span) -> LabeledError {
    LabeledError::new("Missing TLS flag")
        .with_label(format!("Have {have}, missing {missing}"), span)
}

fn read_pem(value: &nu_protocol::Value, kind: &str) -> Result<Vec<u8>, LabeledError> {
    let path = value.to_path()?;
    let pem = std::fs::read(path).map_err(|e| {
        LabeledError::new(format!(
            "{kind} {:?} does not exist",
            value.clone().into_string().unwrap()
        ))
        .with_label(e.to_string(), value.span())
    })?;

    Ok(pem)
}
