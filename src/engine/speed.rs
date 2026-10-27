//! Settled model throughput. No text/tokenizer estimates and no tool wall time.
use crate::util::time;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

const REFRESH_MS: i64 = 30_000;
const WINDOW_MS: i64 = 300_000;
#[derive(Clone, Debug)]
struct Sample { id: String, tokens: i64, ms: i64, at: i64 }
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading { pub value: Option<f64>, pub samples: usize, pub at: i64, pub basis: &'static str }
#[derive(Debug, Default)]
pub struct Tracker {
    model: String,
    samples: VecDeque<Sample>,
    requests: HashMap<String, i64>,
    tools: HashSet<String>,
    start: Option<i64>,
    end: i64,
    native: Option<Reading>,
    native_origin: Option<(i64,i64,usize)>,
    published: Reading,
    refreshed: i64,
    basis: &'static str,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }
fn n(v: &Value, k: &str) -> i64 { v.get(k).and_then(Value::as_i64).unwrap_or(0) }
fn at(v: &Value) -> Option<i64> { v.get("timestamp").and_then(Value::as_str).and_then(time::parse_iso) }

#[derive(Debug)]
pub enum Event {
    Model(String),
    Span { start: i64, end: i64 },
    Commit { id: String, tokens: i64, at: i64 },
    Tool { id: String, running: bool },
}

/// Read only typed identity, timing, usage and tool-boundary metadata.
pub fn codex_event(line: &[u8]) -> Option<Event> {
    let prefix = &line[..line.len().min(2048)];
    let has=|v: &[u8]|super::jsonl::contains(prefix,v);
    let timing=has(b"\"item_completed\"") && [b"\"Reasoning\"".as_slice(),b"\"AgentMessage\"",b"\"Plan\"",b"\"reasoning\"",b"\"agentMessage\"",b"\"agent_message\"",b"\"plan\""].iter().any(|v|has(v));
    let tool=has(b"\"response_item\"") && [b"\"function_call\"".as_slice(),b"\"function_call_output\"",b"\"custom_tool_call\"",b"\"custom_tool_call_output\"",b"\"local_shell_call\"",b"\"local_shell_call_output\""].iter().any(|v|has(v));
    if !(has(b"\"turn_context\"") || has(b"\"token_usage_record\"") || has(b"\"token_count\"") || timing || tool) { return None; }
    let r: Value = serde_json::from_slice(line).ok()?;
    let p = r.get("payload")?;
    match s(&r, "type") {
        "turn_context" => Some(Event::Model(s(p,"model").into())),
        "token_usage_record" => Some(Event::Commit { id: s(p,"response_id").into(), tokens: n(p.get("usage")?,"output_tokens"), at: at(&r)? }),
        "event_msg" if s(p,"type") == "token_count" => {
            let u=p.get("info")?.get("last_token_usage")?;
            Some(Event::Commit { id: format!("usage:{}",at(&r)?), tokens: n(u,"output_tokens"), at: at(&r)? })
        }
        "event_msg" if s(p,"type") == "item_completed" => {
            let item=p.get("item")?;
            if !matches!(s(item,"type").to_ascii_lowercase().as_str(), "reasoning"|"agentmessage"|"agent_message"|"plan") { return None; }
            Some(Event::Span { start: p.get("started_at_ms")?.as_i64()?, end: p.get("completed_at_ms")?.as_i64()? })
        }
        "response_item" => {
            let kind=s(p,"type");
            let running=matches!(kind,"function_call"|"custom_tool_call"|"local_shell_call");
            if !running && !matches!(kind,"function_call_output"|"custom_tool_call_output"|"local_shell_call_output") { return None; }
            let id=s(p,"call_id");
            (!id.is_empty()).then(|| Event::Tool { id:id.into(),running })
        }
        _ => None,
    }
}

impl Tracker {
    pub fn model(&mut self, model: &str) {
        if model.is_empty() || self.model == model { return; }
        self.model=model.into(); self.samples.clear(); self.native=None;
        self.published=Reading::default(); self.refreshed=0;
        self.start=None; self.end=0;
    }
    pub fn boundary(&mut self) { self.tools.clear();self.start=None;self.end=0; }
    pub fn tool(&mut self, id: &str, running: bool) {
        if running { self.tools.insert(id.into()); } else { self.tools.remove(id); }
    }
    pub fn observe(&mut self, event: Event) {
        match event {
            Event::Model(m) => self.model(&m),
            Event::Tool{id,running} => self.tool(&id,running),
            Event::Span{start,end} if start>0 && end>=start => {
                self.start=Some(self.start.map_or(start,|s|s.min(start))); self.end=self.end.max(end);
            }
            Event::Commit{id,tokens,at} => {
                if let Some(start)=self.start.take() {
                    self.add(&id,tokens,self.end-start,at,"output-window");
                }
                self.end=0;
            }
            _ => {}
        }
    }
    fn add(&mut self, id: &str, tokens: i64, ms: i64, at: i64, basis: &'static str) {
        if tokens<=0 || ms<500 { return; }
        self.basis=basis;
        if let Some(old)=self.samples.iter_mut().find(|v|v.id==id) {
            old.tokens=old.tokens.max(tokens);old.ms=old.ms.max(ms);old.at=old.at.max(at);
        } else {
            self.samples.push_back(Sample{id:id.into(),tokens,ms,at});
            while self.samples.len()>5 { self.samples.pop_front(); }
        }
    }
    pub fn claude_line(&mut self, line: &[u8]) {
        if ![b"\"api-request\"".as_slice(),b"\"assistant\"",b"\"tool_result\""].iter().any(|p| super::jsonl::contains(line,p)) { return; }
        let Ok(r)=serde_json::from_slice::<Value>(line) else { return; };
        self.claude_record(&r);
    }
    pub fn claude_record(&mut self, r: &Value) {
        if r.get("isSidechain").and_then(Value::as_bool)==Some(true) { return; }
        match s(&r,"type") {
            "api-request" => {
                if let Some(at)=at(&r) {
                    if self.requests.len()>=16 { self.requests.retain(|_,start|at-*start<WINDOW_MS); }
                    if self.requests.len()<16 { self.requests.insert(s(&r,"id").into(),at); }
                }
            }
            "assistant" => {
                let Some(m)=r.get("message") else { return; };
                if !s(m,"model").starts_with('<') { self.model(s(m,"model")); }
                let reference=s(&r,"requestRef");
                if let (Some(start),Some(end),Some(u))=(self.requests.get(reference).copied(),at(&r),m.get("usage")) {
                    self.add(reference,n(u,"output_tokens"),end-start,end,"api-request");
                }
                if let Some(blocks)=m.get("content").and_then(Value::as_array) {
                    for b in blocks { if s(b,"type")=="tool_use" { self.tool(s(b,"id"),true); } }
                }
            }
            "user" => {
                if let Some(blocks)=r.get("message").and_then(|m|m.get("content")).and_then(Value::as_array) {
                    for b in blocks { if s(b,"type")=="tool_result" { self.tool(s(b,"tool_use_id"),false); } }
                }
            }
            _ => {}
        }
    }
    pub fn dsh(&mut self, p: &super::dsh::Projection, at: i64) {
        if !self.model.is_empty() && self.model!=p.model {
            self.native_origin=p.decode_tokens.zip(p.decode_ms).map(|(t,m)|(t,m,p.decode_steps));
        }
        self.model(&p.model);
        self.tool("projection",p.tools_running);
        self.native=match (p.decode_tokens,p.decode_ms) {
            (Some(tokens),Some(ms)) => {
                let (base_t,base_ms,base_n)=self.native_origin.unwrap_or_default();
                let (tokens,ms)=(tokens-base_t,ms-base_ms);
                (tokens>0 && ms>=500).then(|| Reading { value:Some(tokens as f64*1000.0/ms as f64),samples:p.decode_steps.saturating_sub(base_n),at,basis:"decode" })
            },
            _ => None,
        };
    }
    /// The first valid sample appears immediately; subsequent numerical updates
    /// are coalesced to 30 s. Phase/status zeroing is independent of this timer.
    pub fn refresh(&mut self, now: i64) {
        if self.published.value.is_some() && now-self.refreshed<REFRESH_MS { return; }
        self.samples.retain(|s|now-s.at<WINDOW_MS);
        self.published=if let Some(native)=self.native { native } else {
            let ms:i64=self.samples.iter().map(|s|s.ms).sum();
            let tokens:i64=self.samples.iter().map(|s|s.tokens).sum();
            Reading { value:(ms>0).then(|| tokens as f64*1000.0/ms as f64),samples:self.samples.len(),at:self.samples.back().map(|s|s.at).unwrap_or(0),basis:self.basis }
        };
        self.refreshed=now;
    }
    pub fn reading(&self, status: &str) -> Reading {
        if status!="active" || !self.tools.is_empty() { Reading {value:Some(0.0),..self.published} } else { self.published }
    }
    pub fn raw(&self) -> Reading { self.published }
}

pub fn hydrate_claude(path: &std::path::Path, now: i64) -> Tracker {
    let mut lines=Vec::new();
    let _=super::jsonl::read_tail_reverse(path,1024*1024,1024,|line| { lines.push(line.to_vec());true });
    let mut tracker=Tracker::default();
    for line in lines.into_iter().rev() { tracker.claude_line(&line); }
    tracker.refresh(now);tracker
}

/// Recover only recent timing/usage metadata from a bounded tail, not a full
/// history scan. Reverse reads are folded back in their original order.
pub fn hydrate_codex(path: &std::path::Path, model: &str, now: i64) -> Tracker {
    let mut events=Vec::new();let mut commits=0;
    let _=super::jsonl::read_tail_reverse(path,512*1024,512,|line| {
        if let Some(e)=codex_event(line) { if matches!(e,Event::Commit{..}) { commits+=1; } events.push(e); }
        commits<12
    });
    let mut tracker=Tracker::default();tracker.model(model);
    for e in events.into_iter().rev() { tracker.observe(e); }
    tracker.model(model);tracker.refresh(now);tracker
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weighted_window_refresh_zeroing_and_model_changes() {
        let mut t=Tracker::default();t.model("a");
        t.add("r1",100,1000,1000,"output-window");t.refresh(2000);
        assert_eq!(t.reading("active").value,Some(100.0));
        t.add("r2",100,3000,3000,"output-window");t.refresh(4000);
        assert_eq!(t.raw().value,Some(100.0)); // coalesced; not a live meter
        t.refresh(32_000);assert_eq!(t.raw().value,Some(50.0)); // weighted, not (100+33)/2
        t.tool("a",true);t.tool("b",true);t.tool("a",false);
        assert_eq!(t.reading("active").value,Some(0.0));
        t.tool("b",false);assert_eq!(t.reading("active").value,Some(50.0));
        for state in ["waiting","idle","listening","completed","paused"] { assert_eq!(t.reading(state).value,Some(0.0)); }
        t.model("b");assert_eq!(t.reading("active").value,None);
        t.add("r3",50,1000,33_000,"output-window");t.refresh(33_001);
        t.refresh(334_000);assert_eq!(t.raw().value,None);
    }
    #[test]
    fn codex_pairs_settled_output_with_model_time_and_deduplicates_usage_feeds() {
        let mut t=Tracker::default();
        let events=[
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"Reasoning"},"started_at_ms":1000,"completed_at_ms":3000}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution"},"started_at_ms":3000,"completed_at_ms":23000}}"#,
            r#"{"timestamp":"2026-10-27T01:00:00Z","type":"token_usage_record","payload":{"response_id":"r1","usage":{"output_tokens":200}}}"#,
            r#"{"timestamp":"2026-10-27T01:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"output_tokens":200}}}}"#,
        ];
        for line in events { if let Some(e)=codex_event(line.as_bytes()) { t.observe(e); } }
        t.refresh(time::parse_iso("2026-10-27T01:00:01Z").unwrap());
        assert_eq!(t.raw().value,Some(100.0));assert_eq!(t.raw().samples,1);
        assert!(codex_event(br#"{"type":"response_item","payload":{"type":"message","content":[{"text":"private"}]}}"#).is_none());
    }
    #[test]
    fn claude_matches_request_refs_and_deduplicates_content_blocks() {
        let mut t=Tracker::default();
        t.claude_line(br#"{"type":"api-request","id":"r1","timestamp":"2026-10-27T01:00:00Z"}"#);
        let record=br#"{"type":"assistant","requestRef":"r1","timestamp":"2026-10-27T01:00:02Z","message":{"model":"claude-opus-5-5","usage":{"output_tokens":100},"content":[{"type":"tool_use","id":"t1"}]}}"#;
        t.claude_line(record);t.claude_line(record);
        let now=time::parse_iso("2026-10-27T01:00:03Z").unwrap();t.refresh(now);
        assert_eq!(t.raw().value,Some(50.0));assert_eq!(t.raw().samples,1);
        assert_eq!(t.reading("active").value,Some(0.0));
        t.claude_line(br#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#);
        assert_eq!(t.reading("active").value,Some(50.0));
    }
    #[test]
    fn dsh_uses_native_decode_time_and_resets_at_model_switch() {
        let mut p=super::super::dsh::Projection {model:"deepseek-flash".into(),decode_tokens:Some(1000),decode_ms:Some(5000),decode_steps:3,..Default::default()};
        let mut t=Tracker::default();t.dsh(&p,1000);t.refresh(1001);
        assert_eq!(t.raw().value,Some(200.0));assert_eq!(t.raw().basis,"decode");
        p.model="deepseek-v4-pro".into();t.dsh(&p,2000);t.refresh(2001);assert_eq!(t.raw().value,None);
        p.decode_tokens=Some(1100);p.decode_ms=Some(6000);p.decode_steps=4;
        t.dsh(&p,3000);t.refresh(3001);assert_eq!(t.raw().value,Some(100.0));assert_eq!(t.raw().samples,1);
        p.tools_running=true;t.dsh(&p,4000);assert_eq!(t.reading("active").value,Some(0.0));
    }
}
