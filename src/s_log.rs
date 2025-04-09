// containerspy structured logger

use std::fmt::{Display, Formatter};
use chrono::Utc;

#[allow(dead_code)]
pub fn debug<'a>(args: impl Display, rich: impl IntoIterator<Item = (&'a str, &'a str)>) {
	log_impl(LogLevel::Debug, args.to_string().as_str(), rich);
}

pub fn info<'a>(args: impl Display, rich: impl IntoIterator<Item = (&'a str, &'a str)>) {
	log_impl(LogLevel::Info, args.to_string().as_str(), rich);
}

#[allow(dead_code)]
pub fn warn<'a>(args: impl Display, rich: impl IntoIterator<Item = (&'a str, &'a str)>) {
	log_impl(LogLevel::Warn, args.to_string().as_str(), rich);
}

#[allow(dead_code)]
pub fn error<'a>(args: impl Display, rich: impl IntoIterator<Item = (&'a str, &'a str)>) {
	log_impl(LogLevel::Error, args.to_string().as_str(), rich);
}

#[allow(dead_code)]
pub fn fatal<'a>(args: impl Display, rich: impl IntoIterator<Item = (&'a str, &'a str)>) {
	log_impl(LogLevel::Fatal, args.to_string().as_str(), rich);
}

enum LogLevel {
	Fatal,
	Error,
	Warn,
	Info,
	Debug,
}

impl Display for LogLevel {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_str(
			match self {
				LogLevel::Fatal => "fatal",
				LogLevel::Error => "error",
				LogLevel::Warn => "warn",
				LogLevel::Info => "info",
				LogLevel::Debug => "debug",
			}
		)
	}
}

fn log_impl<'a>(level: LogLevel, msg: &str, rich: impl IntoIterator<Item = (&'a str, &'a str)>) {
	let time = Utc::now();
	let nice_time = time.format("%F %X%.3f");
	let full_time = time.format("%+");

	let mut final_rich = vec![
		("ts", full_time.to_string()),
		("level", level.to_string()),
		("msg", msg.to_string())
	];

	// i don't care anymore, just clone it all temporarily.
	final_rich.extend(rich.into_iter().map(|(a, b)| (a, b.to_string())));

	let mut buf = format!("{nice_time}");
	for (k, v) in final_rich {
		if needs_escaping(k) {
			continue;
		}

		if needs_escaping(&v) {
			buf += &format!(" {k}=\"{}\"", escape(&v));
		} else {
			buf += &format!(" {k}={v}");
		}
	}

	println!("{buf}");
}

static SAFE_ALPHABET: &str = r#"abcdefghijklmnopqrstuvxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_+.,/\\|!@#$%^&*()[]{}"#;

fn needs_escaping(val: &str) -> bool {
	for char in val.chars() {
		if !SAFE_ALPHABET.contains(char) {
			return true
		}
	}
	false
}

fn escape(val: &str) -> String {
	val.replace("\n", "\\n").replace("\\", "\\\\").replace("\"", "\\\"")
}