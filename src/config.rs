use std::sync::LazyLock;

use anyhow::Result;
use confique::Config;
use opentelemetry_otlp::Protocol;

#[derive(Config)]
pub struct CspyConfig {
	#[config(env = "CSPY_DOCKER_SOCKET")]
	pub docker_socket: Option<String>,

	#[config(env = "CSPY_OTLP_PROTO", default = "httpbinary", deserialize_with = crate::config::deser_protocol)]
	pub otlp_protocol: Protocol,

	#[config(env = "CSPY_OTLP_ENDPOINT")]
	pub otlp_endpoint: Option<String>,

	#[config(env = "CSPY_OTLP_INTERVAL")]
	pub otlp_export_interval: Option<u64>,
}

pub static CONFIG: LazyLock<CspyConfig> = LazyLock::new(|| {
	let cfg_loc = std::env::var("CSPY_CONFIG");
	let cfg_loc = cfg_loc.as_deref().ok().unwrap_or("/etc/containerspy/config.json");

	CspyConfig::builder()
		.env()
		.file(cfg_loc)
		.load()
		.unwrap()
});


/// deserialization boilerplate
struct ProtoDeserVisitor;

/// deserialization boilerplate
impl confique::serde::de::Visitor<'_> for ProtoDeserVisitor {
	type Value = Protocol;

	fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
		formatter.write_str(r#""httpbinary", "httpjson", or "grpc"."#)
	}

	fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
		 where
			  E: confique::serde::de::Error, {
		 Ok(match v {
			"httpbinary" => Protocol::HttpBinary,
			"httpjson" => Protocol::HttpJson,
			"grpc" => Protocol::Grpc,
			&_ => return Err(E::custom(format!("{v} is not a valid OTLP protocol, valid options are httpbinary, httpjson, or grpc.")))
		})
	}
}

/// deserialization boilerplate
fn deser_protocol<'de, D: confique::serde::Deserializer<'de>>(d: D) -> Result<Protocol, D::Error> {
	d.deserialize_str(ProtoDeserVisitor)
}
