//! The details window.
//!
//! Rendered in the system webview so it can share the visual language of the
//! project's website rather than inventing a second one, and so the evidence
//! behind the estimate — every sample, every login gap — can be laid out
//! properly instead of squeezed into menu rows.

use gcloud_dot_core::{
    credentials::AdcKind,
    status::{ago, long_duration, AuthState, Level, Status},
    State,
};

/// Everything the page needs, resolved before rendering so the template has no
/// logic in it beyond iteration.
pub struct PanelView {
    pub level: Level,
    pub headline: String,
    pub sub: String,
    pub rows: Vec<(String, String)>,
    pub estimate_hours: f64,
    pub estimate_source: String,
    pub estimate_measured: bool,
    pub samples: Vec<f64>,
    pub logins: Vec<(String, String, String)>,
    pub version: String,
}

pub fn view(status: &Status, state: &State) -> PanelView {
    let now = chrono::Local::now();
    let level = status.level(now);

    let headline = match &status.auth {
        AuthState::Valid => match status.remaining(now) {
            Some(left) if left.num_seconds() >= 0 => long_duration(left),
            Some(_) => "Overdue".to_string(),
            None => "Signed in".to_string(),
        },
        AuthState::Expired => "Signed out".to_string(),
        AuthState::Unknown(_) => "Unknown".to_string(),
    };

    let sub = match &status.auth {
        AuthState::Valid if status.remaining(now).is_some_and(|r| r.num_seconds() >= 0) => {
            "estimated until re-auth".to_string()
        }
        _ => status.summary(now),
    };

    let mut rows = Vec::new();
    if let Some(cfg) = &status.config {
        if let Some(a) = &cfg.account {
            rows.push(("Account".into(), a.clone()));
        }
        if let Some(p) = &cfg.project {
            rows.push(("Project".into(), p.clone()));
        }
        rows.push(("Configuration".into(), cfg.name.clone()));
    }
    if let Some(start) = status.session_start {
        rows.push((
            "Last login".into(),
            format!("{} · {}", start.format("%a %b %-d, %H:%M"), ago(start, now)),
        ));
    }
    if status.auth == AuthState::Valid {
        if let Some(expiry) = status.predicted_expiry() {
            rows.push((
                "Est. re-auth".into(),
                expiry.format("%a %b %-d, %H:%M").to_string(),
            ));
        }
    }
    if let (Some(file), Some(adc)) = (&status.adc_file, &status.adc) {
        let kind = match &file.kind {
            AdcKind::UserCredentials => "user credentials".to_string(),
            AdcKind::ServiceAccount { client_email } => client_email.clone(),
            AdcKind::Other { kind } => kind.clone(),
        };
        let word = match adc {
            AuthState::Valid => "valid",
            AuthState::Expired => "expired",
            AuthState::Unknown(_) => "unknown",
        };
        rows.push(("App default creds".into(), format!("{word} · {kind}")));
    }
    if let Some(checked) = status.checked_at {
        rows.push((
            "Last checked".into(),
            checked.format("%H:%M:%S").to_string(),
        ));
    }

    // Newest first, with the gap to the previous login, which is what the
    // fallback estimator reads when no session has been seen ending.
    let ordered: Vec<_> = state.logins.iter().rev().take(10).collect();
    let logins = ordered
        .iter()
        .enumerate()
        .map(|(i, when)| {
            let gap = ordered
                .get(i + 1)
                .map(|prev| format!("+{:.1}h", (**when - **prev).num_minutes() as f64 / 60.0))
                .unwrap_or_default();
            (
                when.format("%b %-d").to_string(),
                when.format("%H:%M").to_string(),
                gap,
            )
        })
        .collect();

    PanelView {
        level,
        headline,
        sub,
        rows,
        estimate_hours: status.estimate.hours,
        estimate_source: status.estimate.source.label(),
        estimate_measured: status.estimate.source.is_measured(),
        samples: state.samples.clone(),
        logins,
        version: gcloud_dot_core::VERSION.to_string(),
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn rgb_css(level: Level) -> String {
    let (r, g, b) = level.rgb();
    format!("rgb({r},{g},{b})")
}

pub fn html(v: &PanelView) -> String {
    let rows: String = v
        .rows
        .iter()
        .map(|(k, val)| {
            format!(
                "<div class=\"k\">{}</div><div class=\"v\">{}</div>",
                esc(k),
                esc(val)
            )
        })
        .collect();

    // Sample bars, scaled across the observed range rather than from zero:
    // eighteen samples between 15.91 and 16.08 all look identical against a
    // zero baseline, and the spread is the interesting part.
    let bars = if v.samples.len() < 2 {
        String::new()
    } else {
        let min = v.samples.iter().cloned().fold(f64::MAX, f64::min);
        let max = v.samples.iter().cloned().fold(f64::MIN, f64::max);
        let span = (max - min).max(0.0001);
        let inner: String = v
            .samples
            .iter()
            .map(|s| {
                let h = 18.0 + ((s - min) / span) * 34.0;
                format!("<i style=\"height:{h:.1}px\" title=\"{s:.2}h\"></i>")
            })
            .collect();
        format!(
            "<div class=\"spark\">{inner}</div>\
             <div class=\"sparkscale\"><span>{min:.2}h</span><span>{max:.2}h</span></div>"
        )
    };

    let logins: String = v
        .logins
        .iter()
        .map(|(day, time, gap)| {
            format!(
                "<li><span class=\"d\">{}</span><span class=\"t\">{}</span><span class=\"g\">{}</span></li>",
                esc(day),
                esc(time),
                esc(gap)
            )
        })
        .collect();

    let measured_note = if v.estimate_measured {
        "Measured from sessions seen expiring."
    } else {
        "Not yet measured. This is inferred, and will sharpen as sessions are observed ending."
    };

    format!(
        r##"<!-- GCloud Dot details panel -->
<style>
  :root {{
    --wash-a:#d6f0e2; --wash-b:#fdeccd; --page:#f6f7f6;
    --glass:rgba(255,255,255,.66); --glass-line:rgba(255,255,255,.9);
    --text:#12141a; --muted:#585e70; --faint:#868c9c;
    --rule:rgba(0,0,0,.08); --soft:rgba(0,0,0,.045); --soft-line:rgba(0,0,0,.09);
    --shadow:0 10px 30px rgba(30,60,45,.12);
    --accent:{accent};
  }}
  @media (prefers-color-scheme: dark) {{
    :root {{
      --wash-a:#123a2a; --wash-b:#3a2a12; --page:#0a0b0c;
      --glass:rgba(255,255,255,.055); --glass-line:rgba(255,255,255,.1);
      --text:#f3f5f4; --muted:#a6adb8; --faint:#79808c;
      --rule:rgba(255,255,255,.09); --soft:rgba(255,255,255,.07); --soft-line:rgba(255,255,255,.14);
      --shadow:0 18px 44px rgba(0,0,0,.55);
    }}
  }}
  * {{ box-sizing:border-box; }}
  html,body {{ margin:0; height:100%; }}
  body {{
    color:var(--text); background-color:var(--page);
    background-image:
      radial-gradient(70% 52% at 12% 0, var(--wash-a) 0, transparent 62%),
      radial-gradient(62% 48% at 95% 4%, var(--wash-b) 0, transparent 58%);
    font:14px/1.5 -apple-system,BlinkMacSystemFont,"SF Pro Text","Segoe UI",Roboto,sans-serif;
    -webkit-font-smoothing:antialiased; user-select:none;
    /* A column with one scrolling region, rather than one long scrolling page.
       The evidence cards grow without bound — twenty samples and ten logins —
       and at 400 points wide they run past 1100, so on a page that simply
       scrolled, Sign in and Check now would sit permanently below the fold. */
    display:flex; flex-direction:column; overflow:hidden;
  }}
  .scroll {{ flex:1; overflow-y:auto; overflow-x:hidden; padding:18px 18px 4px; }}
  .bottom {{
    padding:11px 18px 14px; border-top:1px solid var(--rule);
    background:var(--glass); backdrop-filter:blur(20px) saturate(170%);
    -webkit-backdrop-filter:blur(20px) saturate(170%);
  }}
  .card {{
    background:var(--glass); border:1px solid var(--glass-line); border-radius:16px;
    backdrop-filter:blur(20px) saturate(170%); -webkit-backdrop-filter:blur(20px) saturate(170%);
    box-shadow:var(--shadow); padding:16px; margin-bottom:12px;
  }}
  .head {{ display:flex; align-items:center; gap:13px; }}
  .dot {{ width:15px; height:15px; border-radius:50%; background:var(--accent);
         box-shadow:0 0 0 4px color-mix(in srgb, var(--accent) 20%, transparent); flex:0 0 auto; }}
  .headline {{ font-size:27px; font-weight:640; letter-spacing:-.022em; line-height:1.1; }}
  .sub {{ color:var(--muted); font-size:12.5px; margin-top:3px; }}
  /* minmax(0,1fr), not 1fr: a grid track's automatic minimum is its content
     size, so a long account address or project id would push the row wider
     than the window and clip every value on the right. */
  .grid {{ display:grid; grid-template-columns:auto minmax(0,1fr); gap:7px 16px;
           margin-top:15px; padding-top:14px; border-top:1px solid var(--rule);
           font-size:13px; }}
  .k {{ color:var(--faint); white-space:nowrap; }}
  .v {{ text-align:right; overflow-wrap:anywhere; font-variant-numeric:tabular-nums; }}
  h2 {{ font-size:11px; text-transform:uppercase; letter-spacing:.075em; color:var(--faint);
        margin:0 0 10px; font-weight:640; }}
  .est {{ display:flex; align-items:baseline; gap:9px; }}
  .est b {{ font-size:23px; font-weight:640; letter-spacing:-.02em; font-variant-numeric:tabular-nums; }}
  .est span {{ color:var(--muted); font-size:12.5px; }}
  .note {{ color:var(--faint); font-size:11.5px; margin-top:8px; line-height:1.45; }}
  .spark {{ display:flex; align-items:flex-end; gap:3px; height:52px; margin-top:13px; }}
  .spark i {{ flex:1; background:var(--accent); opacity:.5; border-radius:2px 2px 0 0; }}
  .spark i:last-child {{ opacity:1; }}
  .sparkscale {{ display:flex; justify-content:space-between; color:var(--faint);
                 font-size:10.5px; margin-top:5px; font-variant-numeric:tabular-nums; }}
  ul {{ list-style:none; margin:0; padding:0; font-size:12.5px; }}
  li {{ display:flex; gap:10px; padding:5px 0; border-bottom:1px solid var(--rule);
        font-variant-numeric:tabular-nums; }}
  li:last-child {{ border-bottom:0; }}
  .d {{ width:52px; color:var(--muted); }}
  .t {{ width:46px; }}
  .g {{ margin-left:auto; color:var(--faint); }}
  .actions {{ display:flex; gap:9px; }}
  button {{
    flex:1; font:inherit; font-weight:560; font-size:13px; padding:9px 12px; cursor:pointer;
    border-radius:10px; border:1px solid var(--soft-line); background:var(--soft); color:var(--text);
    transition:transform .06s ease, background .12s ease;
  }}
  button:hover {{ background:color-mix(in srgb, var(--accent) 12%, var(--soft)); }}
  button:active {{ transform:translateY(1px); }}
  button.primary {{ background:var(--accent); border-color:transparent; color:#fff; }}
  footer {{ text-align:center; color:var(--faint); font-size:11px; margin-top:9px; }}
  footer a {{ color:var(--faint); }}
</style>

<div class="scroll">
<div class="card">
  <div class="head">
    <div class="dot"></div>
    <div>
      <div class="headline">{headline}</div>
      <div class="sub">{sub}</div>
    </div>
  </div>
  <div class="grid">{rows}</div>
</div>

<div class="card">
  <h2>Session length</h2>
  <div class="est"><b>{hours:.2}h</b><span>{source}</span></div>
  {bars}
  <div class="note">{measured_note}</div>
</div>

<div class="card">
  <h2>Recent logins</h2>
  <ul>{logins}</ul>
</div>
</div>

<div class="bottom">
  <div class="actions">
    <button class="primary" onclick="send('login')">Sign in</button>
    <button onclick="send('check')">Check now</button>
  </div>
  <footer>GCloud Dot {version} · <a href="#" onclick="send('website');return false">website</a></footer>
</div>

<script>
  function send(action) {{ window.ipc.postMessage(JSON.stringify({{action}})); }}
</script>
"##,
        accent = rgb_css(v.level),
        headline = esc(&v.headline),
        sub = esc(&v.sub),
        rows = rows,
        hours = v.estimate_hours,
        source = esc(&v.estimate_source),
        bars = bars,
        measured_note = measured_note,
        logins = logins,
        version = esc(&v.version),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local};
    use gcloud_dot_core::{
        config::ActiveConfig,
        estimate::{Estimate, EstimateSource},
    };

    fn fixture() -> (Status, State) {
        let mut state = State::default();
        let base = Local::now() - Duration::hours(40);
        for i in 0..3 {
            state.logins.push(base + Duration::hours(i * 16));
        }
        for s in [15.91, 16.07, 16.06] {
            state.record_sample(s);
        }
        let status = Status {
            gcloud_found: true,
            auth: AuthState::Valid,
            config: Some(ActiveConfig {
                name: "default".into(),
                account: Some("nic@glazkov.com".into()),
                project: Some("my-project".into()),
            }),
            session_start: state.last_login(),
            estimate: Estimate {
                hours: 16.06,
                source: EstimateSource::Observed { count: 3 },
            },
            checked_at: Some(Local::now()),
            ..Default::default()
        };
        (status, state)
    }

    #[test]
    fn renders_the_account_and_evidence() {
        let (status, state) = fixture();
        let page = html(&view(&status, &state));
        assert!(page.contains("nic@glazkov.com"));
        assert!(page.contains("my-project"));
        assert!(page.contains("16.06"));
        assert!(page.contains("Measured from sessions"));
    }

    #[test]
    fn says_plainly_when_the_estimate_is_a_guess() {
        let (mut status, state) = fixture();
        status.estimate = Estimate {
            hours: 20.0,
            source: EstimateSource::Default,
        };
        let page = html(&view(&status, &state));
        assert!(
            page.contains("Not yet measured"),
            "must not imply measurement"
        );
    }

    #[test]
    fn escapes_values_that_come_from_the_environment() {
        // A project id cannot contain a bracket, but a configuration name can
        // be anything a user typed, and it lands straight in the markup.
        let (mut status, state) = fixture();
        status.config.as_mut().unwrap().name = "<img src=x onerror=alert(1)>".into();
        let page = html(&view(&status, &state));
        assert!(
            !page.contains("<img src=x"),
            "unescaped value reached the page"
        );
        assert!(page.contains("&lt;img"));
    }

    #[test]
    fn a_signed_out_panel_does_not_show_a_countdown() {
        let (mut status, state) = fixture();
        status.auth = AuthState::Expired;
        let v = view(&status, &state);
        assert_eq!(v.headline, "Signed out");
        assert!(v.rows.iter().all(|(k, _)| k != "Est. re-auth"));
    }

    #[test]
    fn gaps_are_computed_between_consecutive_logins() {
        let (status, state) = fixture();
        let v = view(&status, &state);
        assert_eq!(v.logins.len(), 3);
        assert_eq!(v.logins[0].2, "+16.0h");
        // The oldest entry has nothing before it to measure against.
        assert_eq!(v.logins[2].2, "");
    }

    #[test]
    fn one_sample_draws_no_sparkline() {
        let (status, mut state) = fixture();
        state.samples = vec![16.0];
        assert!(!html(&view(&status, &state)).contains("class=\"spark\""));
    }
}
