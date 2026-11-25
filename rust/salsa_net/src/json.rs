// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn obj(pairs: Vec<(&str, JsonValue)>) -> Self {
        Self::Object(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            Self::Object(entries) => entries.iter().rfind(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut JsonValue> {
        match self {
            Self::Object(entries) => entries.iter_mut().rfind(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn set(&mut self, key: impl Into<String>, val: JsonValue) {
        let k = key.into();
        if let Self::Object(entries) = self {
            if let Some(slot) = entries.iter_mut().rfind(|(ek, _)| *ek == k) {
                slot.1 = val;
            } else {
                entries.push((k, val));
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<JsonValue> {
        if let Self::Object(entries) = self {
            let mut removed = None;
            let mut idx = entries.len();
            while idx > 0 {
                idx -= 1;
                if entries[idx].0 == key {
                    let (_, val) = entries.remove(idx);
                    if removed.is_none() {
                        removed = Some(val);
                    }
                }
            }
            return removed;
        }
        None
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) if f.is_finite() => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Int(i) if *i >= 0 => Some(*i as u64),
            Self::Float(f) if f.is_finite() && *f >= 0.0 => Some(*f as u64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            Self::Object(o) => Some(o.as_slice()),
            _ => None,
        }
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let mut p = Parser {
            b: input.as_bytes(),
            pos: 0,
            depth: 0,
        };
        let v = p.parse_value()?;
        p.skip_ws();
        if p.pos != p.b.len() {
            return Err(format!("trailing bytes at offset {}", p.pos));
        }
        Ok(v)
    }

    pub fn to_json_compact(&self) -> String {
        let mut out = String::new();
        self.write_fmt(&mut out, 0, None);
        out
    }

    pub fn to_json_pretty(&self, indent: usize) -> String {
        let mut out = String::new();
        self.write_fmt(&mut out, 0, Some(indent));
        out
    }

    fn write_fmt(&self, out: &mut String, depth: usize, indent: Option<usize>) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Self::Float(f) => {
                if !f.is_finite() {
                    out.push_str("null");
                } else {
                    let s = format!("{f}");
                    out.push_str(&s);
                    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                        out.push_str(".0");
                    }
                }
            }
            Self::Str(s) => write_json_str(out, s),
            Self::Array(arr) => {
                if arr.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, item) in arr.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if let Some(ind) = indent {
                        out.push('\n');
                        for _ in 0..((depth + 1) * ind) {
                            out.push(' ');
                        }
                    }
                    item.write_fmt(out, depth + 1, indent);
                }
                if let Some(ind) = indent {
                    out.push('\n');
                    for _ in 0..(depth * ind) {
                        out.push(' ');
                    }
                }
                out.push(']');
            }
            Self::Object(entries) => {
                if entries.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if let Some(ind) = indent {
                        out.push('\n');
                        for _ in 0..((depth + 1) * ind) {
                            out.push(' ');
                        }
                    }
                    write_json_str(out, k);
                    if indent.is_some() {
                        out.push_str(": ");
                    } else {
                        out.push(':');
                    }
                    v.write_fmt(out, depth + 1, indent);
                }
                if let Some(ind) = indent {
                    out.push('\n');
                    for _ in 0..(depth * ind) {
                        out.push(' ');
                    }
                }
                out.push('}');
            }
        }
    }
}

fn write_json_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.b.len() && matches!(self.b[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        let ch = self.peek().ok_or_else(|| "unexpected EOF".to_string())?;
        match ch {
            b'n' => {
                if self.b[self.pos..].starts_with(b"null") {
                    self.pos += 4;
                    Ok(JsonValue::Null)
                } else {
                    Err("invalid null".into())
                }
            }
            b't' => {
                if self.b[self.pos..].starts_with(b"true") {
                    self.pos += 4;
                    Ok(JsonValue::Bool(true))
                } else {
                    Err("invalid true".into())
                }
            }
            b'f' => {
                if self.b[self.pos..].starts_with(b"false") {
                    self.pos += 5;
                    Ok(JsonValue::Bool(false))
                } else {
                    Err("invalid false".into())
                }
            }
            b'"' => Ok(JsonValue::Str(self.parse_string()?)),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            c => Err(format!("unexpected byte '{}' at {}", c as char, self.pos)),
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 > self.b.len() {
            return Err("truncated \\u escape".into());
        }
        let s = std::str::from_utf8(&self.b[self.pos..self.pos + 4])
            .map_err(|_| "invalid utf8 in \\u escape".to_string())?;
        self.pos += 4;
        u16::from_str_radix(s, 16).map_err(|_| format!("invalid hex '{s}'"))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.b.len() {
            let byte = self.b[self.pos];
            if byte == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if byte == b'\\' {
                self.pos += 1;
                let esc = *self
                    .b
                    .get(self.pos)
                    .ok_or_else(|| "unterminated escape".to_string())?;
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&code) {
                            if self.pos + 2 <= self.b.len()
                                && self.b[self.pos] == b'\\'
                                && self.b[self.pos + 1] == b'u'
                            {
                                let saved = self.pos;
                                self.pos += 2;
                                let lo = self.parse_hex4()?;
                                if (0xDC00..=0xDFFF).contains(&lo) {
                                    let combined =
                                        0x10000 + (((code as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                                    if let Some(c) = char::from_u32(combined) {
                                        out.push(c);
                                        continue;
                                    }
                                }
                                self.pos = saved;
                            }
                            out.push(char::REPLACEMENT_CHARACTER);
                        } else if let Some(c) = char::from_u32(code as u32) {
                            out.push(c);
                        } else {
                            out.push(char::REPLACEMENT_CHARACTER);
                        }
                    }
                    other => return Err(format!("invalid escape \\{}", other as char)),
                }
            } else if byte < 0x80 {
                out.push(byte as char);
                self.pos += 1;
            } else {
                let s = std::str::from_utf8(&self.b[self.pos..])
                    .map_err(|_| "invalid utf8 in json string".to_string())?;
                let ch = s.chars().next().unwrap();
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err("unterminated string".into())
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        let mut is_float = false;
        if self.b[self.pos] == b'-' {
            self.pos += 1;
        }
        while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos < self.b.len() && self.b[self.pos] == b'.' {
            is_float = true;
            self.pos += 1;
            while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if self.pos < self.b.len() && matches!(self.b[self.pos], b'e' | b'E') {
            is_float = true;
            self.pos += 1;
            if self.pos < self.b.len() && matches!(self.b[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        let slice = std::str::from_utf8(&self.b[start..self.pos])
            .map_err(|_| "invalid number".to_string())?;
        if is_float {
            let f: f64 = slice.parse().map_err(|_| format!("bad float '{slice}'"))?;
            Ok(JsonValue::Float(f))
        } else if let Ok(i) = slice.parse::<i64>() {
            Ok(JsonValue::Int(i))
        } else {
            let f: f64 = slice.parse().map_err(|_| format!("bad number '{slice}'"))?;
            Ok(JsonValue::Float(f))
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        if self.depth >= MAX_DEPTH {
            return Err("json max nesting depth exceeded".into());
        }
        self.depth += 1;
        let res = (|| -> Result<JsonValue, String> {
            self.pos += 1;
            let mut items = Vec::new();
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(JsonValue::Array(items));
            }
            loop {
                items.push(self.parse_value()?);
                self.skip_ws();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b']') => {
                        self.pos += 1;
                        return Ok(JsonValue::Array(items));
                    }
                    _ => return Err(format!("expected ',' or ']' at {}", self.pos)),
                }
            }
        })();
        self.depth -= 1;
        res
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        if self.depth >= MAX_DEPTH {
            return Err("json max nesting depth exceeded".into());
        }
        self.depth += 1;
        let res = (|| -> Result<JsonValue, String> {
            self.pos += 1;
            let mut entries = Vec::new();
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return Ok(JsonValue::Object(entries));
            }
            loop {
                self.skip_ws();
                if self.peek() != Some(b'"') {
                    return Err(format!("expected object key string at {}", self.pos));
                }
                let key = self.parse_string()?;
                self.skip_ws();
                if self.peek() != Some(b':') {
                    return Err(format!("expected ':' at {}", self.pos));
                }
                self.pos += 1;
                let val = self.parse_value()?;
                entries.push((key, val));
                self.skip_ws();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b'}') => {
                        self.pos += 1;
                        return Ok(JsonValue::Object(entries));
                    }
                    _ => return Err(format!("expected ',' or '}}' at {}", self.pos)),
                }
            }
        })();
        self.depth -= 1;
        res
    }
}

#[cfg(test)]
mod tests {
    use super::JsonValue;

    #[test]
    fn json_roundtrip_and_escapes() {
        let src = r#"{"a": 1, "b": -3.5, "c": [true, false, null, "hi\n\"🌍\"\u0041"], "nested": {"k": "v"}}"#;
        let v = JsonValue::parse(src).expect("parse json");
        assert_eq!(v.get("a").and_then(|x| x.as_i64()), Some(1));
        assert_eq!(v.get("b").and_then(|x| x.as_f64()), Some(-3.5));
        let arr = v.get("c").and_then(|x| x.as_array()).unwrap();
        assert_eq!(arr[0].as_bool(), Some(true));
        assert_eq!(arr[2], JsonValue::Null);
        assert_eq!(arr[3].as_str(), Some("hi\n\"🌍\"A"));
        assert_eq!(
            v.get("nested").and_then(|x| x.get("k")).and_then(|x| x.as_str()),
            Some("v")
        );

        let pretty = v.to_json_pretty(2);
        let reparsed = JsonValue::parse(&pretty).expect("reparse pretty");
        assert_eq!(v, reparsed);
    }

    #[test]
    fn json_depth_limit_and_lone_surrogate() {
        let deep = format!("{}{}", "[".repeat(140), "]".repeat(140));
        assert!(JsonValue::parse(&deep).is_err());

        let lone = JsonValue::parse(r#""\uD800\u0041""#).expect("lone high surrogate");
        assert_eq!(lone.as_str(), Some("\u{FFFD}A"));
    }
}
