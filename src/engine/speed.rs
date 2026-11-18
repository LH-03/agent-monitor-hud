//! Settled response throughput. No text/tokenizer estimates or tool wall time.
use crate::util::time;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};

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
    tools: HashSet<String>,
    response_started: Option<i64>,
    tool_call_at: Option<i64>,
    native_usage: bool,
    last_total: Option<i64>,
    responses: VecDeque<String>,
    native: Option<Reading>,
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
    Boundary(Option<i64>),
    Commit { id: String, tokens: i64, at: i64, native: bool, total: Option<i64> },
    Tool { id: String, running: bool, at: Option<i64> },
}

/// Read only typed identity, timing, usage and tool-boundary metadata.
pub fn codex_event(line: &[u8]) -> Option<Event> {
    let prefix = &line[..line.len().min(2048)];
    let has=|v: &[u8]|super::jsonl::contains(prefix,v);
    let tool=has(b"\"response_item\"") && [b"\"function_call\"".as_slice(),b"\"function_call_output\"",b"\"custom_tool_call\"",b"\"custom_tool_call_output\"",b"\"local_shell_call\"",b"\"local_shell_call_output\""].iter().any(|v|has(v));
    let boundary=[b"\"task_started\"".as_slice(),b"\"task_complete\"",b"\"turn_aborted\""].iter().any(|v|has(v));
    if !(has(b"\"turn_context\"") || has(b"\"token_usage_record\"") || has(b"\"token_count\"") || tool || boundary) { return None; }
    let r: Value = serde_json::from_slice(line).ok()?;
    let p = r.get("payload")?;
    match s(&r, "type") {
        "turn_context" => Some(Event::Model(s(p,"model").into())),
        "token_usage_record" => Some(Event::Commit { id: s(p,"response_id").into(), tokens: n(p.get("usage")?,"output_tokens"), at: at(&r)?, native:true, total:p.get("thread_token_usage").and_then(|u|u.get("output_tokens")).and_then(Value::as_i64) }),
        "event_msg" if s(p,"type") == "task_started" => Some(Event::Boundary(Some(at(&r)?))),
        "event_msg" if matches!(s(p,"type"),"task_complete"|"turn_aborted") => Some(Event::Boundary(None)),
        "event_msg" if s(p,"type") == "token_count" => {
            let u=p.get("info")?.get("last_token_usage")?;
            let total=p.get("info")?.get("total_token_usage")?.get("output_tokens")?.as_i64()?;
            Some(Event::Commit { id: format!("usage:{total}"), tokens: n(u,"output_tokens"), at: at(&r)?, native:false, total:Some(total) })
        }
        "response_item" => {
            let kind=s(p,"type");
            let running=matches!(kind,"function_call"|"custom_tool_call"|"local_shell_call");
            if !running && !matches!(kind,"function_call_output"|"custom_tool_call_output"|"local_shell_call_output") { return None; }
            let id=s(p,"call_id");
            (!id.is_empty() || (running && at(&r).is_some())).then(|| Event::Tool { id:id.into(),running,at:at(&r) })
        }
        _ => None,
    }
}

impl Tracker {
    pub fn model(&mut self, model: &str) {
        if model.is_empty() || self.model == model { return; }
        self.model=model.into(); self.samples.clear(); self.native=None;
        self.published=Reading::default(); self.refreshed=0;
    }
    fn boundary(&mut self) { self.tools.clear();self.response_started=None;self.tool_call_at=None; }
    pub fn tool(&mut self, id: &str, running: bool) {
        if running { self.tools.insert(id.into()); } else { self.tools.remove(id); }
    }
    pub fn observe(&mut self, event: Event) {
        match event {
            Event::Model(m) => self.model(&m),
            Event::Boundary(start) => { self.boundary();self.response_started=start; },
            Event::Tool{id,running,at} => {
                if running {
                    if let Some(at)=at { self.tool_call_at=Some(self.tool_call_at.unwrap_or(at).max(at)); }
                }
                if !id.is_empty() { self.tool(&id,running); }
                if !running && self.tools.is_empty() && self.response_started.is_none() { self.response_started=at; }
            },
            Event::Commit{id,tokens,at,native,total} => {
                if native {
                    self.native_usage=true;
                    if id.is_empty() || self.responses.contains(&id) { return; }
                    self.responses.push_back(id.clone());
                    while self.responses.len()>16 { self.responses.pop_front(); }
                } else if self.native_usage || total.is_none() || total==self.last_total { return; }
                self.last_total=total;
                // Item timestamps describe visible summaries, not all hidden
                // reasoning tokens. Start at the turn/tool-return boundary and
                // end at the response's own usage record. The legacy aggregate
                // is emitted after tools, so stop it at tool argument completion.
                // Includes first-token latency; missing starts stay unknown.
                let end=if native { at } else { self.tool_call_at.filter(|end|*end<=at).unwrap_or(at) };
                if let Some(start)=self.response_started {
                    self.add(&id,tokens,end-start,at,"response-window");
                }
                self.tool_call_at=None;
                self.response_started=if self.tools.is_empty() { Some(at) } else { None };
            }
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
    pub fn dsh(&mut self, p: &super::dsh::Projection, at: i64) {
        self.model(&p.model);
        // Match the Harness session-statistics pill: its entire session's
        // decode totals, even while a tool is running or after a model switch.
        let value=match (p.decode_tokens,p.decode_ms) {
            (Some(tokens),Some(ms)) if tokens>=0 && ms>0.0 => Some(tokens as f64*1000.0/ms),
            _ => None,
        };
        // DSH exposes the number of TTFT observations, not the number of
        // decode samples; don't present that unrelated count as TPS samples.
        let reading=Reading { value,samples:0,at,basis:"decode" };
        self.native=Some(reading);
        self.published=reading;
    }
    /// The first valid sample appears immediately; subsequent numerical updates
    /// are coalesced to 30 s. Phase/status zeroing is independent of this timer.
    pub fn refresh(&mut self, now: i64) {
        if let Some(native)=self.native { self.published=native; return; }
        if self.published.value.is_some() && now-self.refreshed<REFRESH_MS { return; }
        self.samples.retain(|s|now-s.at<WINDOW_MS);
        self.published={
            let ms:i64=self.samples.iter().map(|s|s.ms).sum();
            let tokens:i64=self.samples.iter().map(|s|s.tokens).sum();
            Reading { value:(ms>0).then(|| tokens as f64*1000.0/ms as f64),samples:self.samples.len(),at:self.samples.back().map(|s|s.at).unwrap_or(0),basis:self.basis }
        };
        self.refreshed=now;
    }
    pub fn reading(&self, status: &str) -> Reading {
        if self.native.is_none() && (status!="active" || !self.tools.is_empty()) { Reading {value:Some(0.0),..self.published} } else { self.published }
    }
    pub fn raw(&self) -> Reading { self.published }
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
    fn codex_duplicate_usage_does_not_restart_the_next_response_clock() {
        let mut t=Tracker::default();
        t.observe(Event::Boundary(Some(1000)));
        t.observe(Event::Commit{id:"r1".into(),tokens:200,at:3100,native:true,total:Some(200)});
        t.observe(Event::Commit{id:"r1".into(),tokens:200,at:3200,native:true,total:Some(200)});
        t.observe(Event::Commit{id:"usage:200".into(),tokens:200,at:8000,native:false,total:Some(200)});
        t.observe(Event::Commit{id:"r2".into(),tokens:1000,at:13200,native:true,total:Some(1200)});
        t.refresh(13201);
        assert!((t.raw().value.unwrap()-1200000.0/12200.0).abs()<1e-8);assert_eq!(t.raw().samples,2);
        assert!(codex_event(br#"{"type":"response_item","payload":{"type":"message","content":[{"text":"private"}]}}"#).is_none());
        assert!(codex_event(br#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution"},"completed_at_ms":23000}}"#).is_none());
    }
    #[test]
    fn codex_counts_tool_arguments_but_not_tool_execution() {
        let mut t=Tracker::default();
        t.observe(Event::Boundary(Some(1000)));
        t.observe(Event::Tool{id:"call".into(),running:true,at:Some(7000)});
        t.observe(Event::Tool{id:"parallel".into(),running:true,at:Some(7500)});
        t.observe(Event::Commit{id:"response".into(),tokens:650,at:7600,native:true,total:Some(650)});
        t.refresh(7601);assert!((t.raw().value.unwrap()-650000.0/6600.0).abs()<1e-8);
        assert_eq!(t.reading("active").value,Some(0.0));
        t.observe(Event::Tool{id:"call".into(),running:false,at:Some(17000)});
        assert_eq!(t.reading("active").value,Some(0.0));
        t.observe(Event::Tool{id:"parallel".into(),running:false,at:Some(19500)});
        t.observe(Event::Commit{id:"usage:650".into(),tokens:650,at:20000,native:false,total:Some(650)});
        t.observe(Event::Commit{id:"next".into(),tokens:300,at:22600,native:true,total:Some(950)});
        t.refresh(38000);
        assert!((t.raw().value.unwrap()-950000.0/9700.0).abs()<1e-8);assert_eq!(t.raw().samples,2);
        let event=codex_event(br#"{"timestamp":"2026-10-27T01:00:05Z","type":"response_item","payload":{"type":"function_call","call_id":"call"}}"#).unwrap();
        assert!(matches!(event,Event::Tool{running:true,at:Some(_),..}));
    }
    #[test]
    fn codex_hidden_reasoning_does_not_shrink_response_time() {
        let mut t=Tracker::default();
        let base=time::parse_iso("2026-11-17T00:00:00Z").unwrap();
        for line in [
            serde_json::json!({"timestamp":"2026-11-17T00:00:01Z","type":"event_msg","payload":{"type":"task_started"}}),
            serde_json::json!({"timestamp":"2026-11-17T00:00:09Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"Reasoning"},"started_at_ms":base+9000,"completed_at_ms":base+9000}}),
            serde_json::json!({"timestamp":"2026-11-17T00:00:11Z","type":"response_item","payload":{"type":"function_call","call_id":"call"}}),
            serde_json::json!({"timestamp":"2026-11-17T00:00:11.200Z","type":"token_usage_record","payload":{"response_id":"r1","usage":{"output_tokens":1000},"thread_token_usage":{"output_tokens":1000}}}),
        ] { if let Some(e)=codex_event(&serde_json::to_vec(&line).unwrap()) { t.observe(e); } }
        t.refresh(base+12000);
        assert!((t.raw().value.unwrap()-1000000.0/10200.0).abs()<1e-8); // 10.2 s, not the 2 s summary/arguments tail.
        assert_eq!(t.raw().basis,"response-window");
    }
    #[test]
    fn codex_legacy_usage_excludes_tools_and_user_pauses() {
        let mut t=Tracker::default();
        t.observe(Event::Boundary(Some(1000)));
        t.model("a"); // turn_context follows task_started, so retain the start.
        t.observe(Event::Tool{id:"call".into(),running:true,at:Some(7000)});
        t.observe(Event::Tool{id:"call".into(),running:false,at:Some(17000)});
        t.observe(Event::Commit{id:"usage:600".into(),tokens:600,at:17100,native:false,total:Some(600)});
        t.refresh(17101);assert_eq!(t.raw().value,Some(100.0));
        t.observe(Event::Commit{id:"usage:600".into(),tokens:600,at:18000,native:false,total:Some(600)});
        t.observe(Event::Boundary(None));
        t.observe(Event::Boundary(Some(100000)));
        t.observe(Event::Commit{id:"usage:800".into(),tokens:200,at:102100,native:false,total:Some(800)});
        t.refresh(102101);assert!((t.raw().value.unwrap()-800000.0/8100.0).abs()<1e-8);assert_eq!(t.raw().samples,2);
        let event=codex_event(br#"{"timestamp":"2026-11-17T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"output_tokens":600},"total_token_usage":{"output_tokens":600}}}}"#).unwrap();
        assert!(matches!(event,Event::Commit{native:false,total:Some(600),tokens:600,..}));
    }
    #[test]
    fn codex_tail_without_a_start_stays_unknown_until_a_tool_return() {
        let mut t=Tracker::default();
        t.observe(Event::Tool{id:"call".into(),running:true,at:Some(3000)});
        t.observe(Event::Commit{id:"incomplete".into(),tokens:1000,at:3100,native:true,total:None});
        t.refresh(3101);assert_eq!(t.raw().value,None);
        t.observe(Event::Tool{id:"call".into(),running:false,at:Some(13000)});
        t.observe(Event::Tool{id:"next".into(),running:true,at:Some(15000)});
        t.observe(Event::Commit{id:"complete".into(),tokens:200,at:15100,native:true,total:None});
        t.refresh(15101);assert!((t.raw().value.unwrap()-200000.0/2100.0).abs()<1e-8);assert_eq!(t.raw().samples,1);
    }
    #[test]
    fn dsh_matches_harness_session_totals_across_phases_and_model_changes() {
        let mut p=super::super::dsh::Projection {model:"deepseek-flash".into(),decode_tokens:Some(1000),decode_ms:Some(5000.0),..Default::default()};
        let mut t=Tracker::default();t.dsh(&p,1000);t.refresh(1001);
        assert_eq!(t.raw().value,Some(200.0));assert_eq!(t.raw().basis,"decode");
        p.model="deepseek-v4-pro".into();t.dsh(&p,2000);t.refresh(2001);assert_eq!(t.raw().value,Some(200.0));
        p.decode_tokens=Some(1200);p.decode_ms=Some(6000.0);
        t.dsh(&p,3000);t.refresh(3001);assert_eq!(t.raw().value,Some(200.0));assert_eq!(t.raw().samples,0);
        t.tool("pending",true);
        p.decode_tokens=Some(1500);t.dsh(&p,4000);t.refresh(4001);
        assert_eq!(t.reading("active").value,Some(250.0));assert_eq!(t.reading("waiting").value,Some(250.0));
        p.decode_ms=Some(0.0);t.dsh(&p,5000);t.refresh(5001);assert_eq!(t.reading("idle").value,None);
    }
}
