#!/usr/bin/env python3
"""Temporary instrumentation only: apply to an extracted source baseline, never the checkout."""
import argparse
import pathlib


def instrument(root, runtime_only=False):
    def edit(name, old, new):
        if runtime_only and name != "apps/backend/src/runtime/curation/field_scheduler.rs":
            return
        path = root / name
        source = path.read_text()
        assert source.count(old) == 1, (name, source.count(old), old[:60])
        path.write_text(source.replace(old, new))

    core = "modules/knowledge/core/src/candidates/batch_writes.rs"
    edit(core, "        let mut tx = self.store.begin()", "        let probe = std::time::Instant::now();\n        let mut marks = Vec::new();\n        let mut tx = self.store.begin()")
    for old, name in [
        ("        tx.lock_project(project)", "begin"),
        ("        let mut candidate = if projection_only", "lock"),
        ("        running(&candidate)?;", "load"),
        ("        let mut preview = SnapshotTx", "guards"),
        ("        commands(\n", "clone"),
        ("        valid_names(&preview.view)?;", "commands"),
        ("        candidate.view = preview.view;", "names"),
        ("        if projection_only {", "bookkeeping"),
        ("        tx.commit().await.map_err(store_error)?;", "put"),
    ]:
        edit(core, old, f'        marks.push(("{name}", probe.elapsed().as_secs_f64()*1000.0));\n'+old)
    edit(core, "        Ok(candidate)", '''        marks.push(("commit", probe.elapsed().as_secs_f64()*1000.0));
        if std::env::var_os("SAVE_PROBE_DETAIL").is_some() {
            eprintln!("SAVE_PROBE {}", serde_json::json!({"kind":"append","candidate":id,"commands":proposed.len(),"marks_ms":marks}));
        }
        Ok(candidate)''')
    adapter = "modules/knowledge/adapter/src/postgres/candidates.rs"
    edit(adapter, '        let row=sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches,progress,parent,applied_rounds,reviewed_by,operations,NULL::jsonb AS execution_input FROM candidates WHERE project_id=$1 AND id=$2")', '        let probe = std::time::Instant::now();\n        let row=sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches,progress,parent,applied_rounds,reviewed_by,operations,NULL::jsonb AS execution_input FROM candidates WHERE project_id=$1 AND id=$2")')
    edit(adapter, '''        row.as_ref().map(candidate).transpose()
    }
    pub(super) async fn db_put_candidate_projection''', '''        let fetched = probe.elapsed().as_secs_f64()*1000.0;
        let result = row.as_ref().map(candidate).transpose();
        if std::env::var_os("SAVE_PROBE_DETAIL").is_some() {
            eprintln!("SAVE_PROBE {}", serde_json::json!({"kind":"read","candidate":key,"fetch_ms":fetched,"decode_ms":probe.elapsed().as_secs_f64()*1000.0-fetched}));
        }
        result
    }
    pub(super) async fn db_put_candidate_projection''')
    edit(adapter, '''        let result=sqlx::query("UPDATE candidates SET state=$3,updated_at=$4,view=$5,batches=$6,progress=$7,applied_rounds=$8,reviewed_by=$9,operations=$10 WHERE project_id=$1 AND id=$2")
            .bind(uuid(c.project_id)).bind(c.id).bind(text(c.state)).bind(c.updated_at)
            .bind(json(&c.view)?).bind(json(&c.batches)?).bind(&c.progress)
            .bind(json(&c.applied_rounds)?).bind(c.reviewed_by.map(uuid)).bind(json(&c.operations)?)
            .execute(&mut *self.tx).await.map_err(unavailable)?;''', '''        let probe = std::time::Instant::now();
        let mut values_ms = 0.0;
        let mut bind_ms = 0.0;
        let mut query=sqlx::query("UPDATE candidates SET state=$3,updated_at=$4,view=$5,batches=$6,progress=$7,applied_rounds=$8,reviewed_by=$9,operations=$10 WHERE project_id=$1 AND id=$2")
            .bind(uuid(c.project_id)).bind(c.id).bind(text(c.state)).bind(c.updated_at);
        macro_rules! measured_bind {
            ($value:expr) => {{
                let timer = std::time::Instant::now();
                let value = $value;
                values_ms += timer.elapsed().as_secs_f64()*1000.0;
                let timer = std::time::Instant::now();
                query = query.bind(value);
                bind_ms += timer.elapsed().as_secs_f64()*1000.0;
            }};
        }
        measured_bind!(json(&c.view)?);
        measured_bind!(json(&c.batches)?);
        measured_bind!(&c.progress);
        measured_bind!(json(&c.applied_rounds)?);
        measured_bind!(c.reviewed_by.map(uuid));
        measured_bind!(json(&c.operations)?);
        let encoded_ms = probe.elapsed().as_secs_f64()*1000.0;
        let result = query.execute(&mut *self.tx).await.map_err(unavailable)?;
        if std::env::var_os("SAVE_PROBE_DETAIL").is_some() {
            eprintln!("SAVE_PROBE {}", serde_json::json!({"kind":"write","candidate":c.id,"values_ms":values_ms,"bind_ms":bind_ms,"execute_ms":probe.elapsed().as_secs_f64()*1000.0-encoded_ms}));
        }''')
    runtime = "apps/backend/src/runtime/curation/field_scheduler.rs"
    edit(runtime, "            s.knowledge.apply(round, &commands).await?;", '''            let repeats = std::env::var("SAVE_PROBE_REPEATS").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
            let command_json = serde_json::to_string(&commands).expect("probe commands");
            let probe_dir = std::env::var("SAVE_PROBE_DIR").ok();
            if let Some(dir) = &probe_dir {
                std::fs::write(format!("{dir}/ready.json"), &command_json).expect("probe ready");
                while !std::path::Path::new(&format!("{dir}/go")).exists() { tokio::time::sleep(std::time::Duration::from_millis(20)).await; }
                commands = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/commands.json")).expect("probe commands file")).expect("probe valid Command");
            }
            let command_json = serde_json::to_string(&commands).expect("probe fixed commands");
            for sample in 0..repeats {
                let timer = Instant::now();
                s.knowledge.apply(round, &commands).await?;
                let apply_ms = timer.elapsed().as_secs_f64()*1000.0;
                if repeats > 1 {
                    s.checkpoint(project, job, nexofolio_contracts::knowledge::CandidateState::Running).await?;
                    eprintln!("SAVE_SAMPLE {}", serde_json::json!({"sample":sample,"task":task,"commands":commands.len(),"apply_ms":apply_ms,"apply_checkpoint_ms":timer.elapsed().as_secs_f64()*1000.0,"command_json":if sample==0 {Some(&command_json)} else {None}}));
                }
            }
            if let Some(dir) = &probe_dir {
                std::fs::write(format!("{dir}/done"), "done").expect("probe done");
                while !std::path::Path::new(&format!("{dir}/reject")).exists() { tokio::time::sleep(std::time::Duration::from_millis(20)).await; }
                let rejected = s.knowledge.apply(round, &commands).await.is_err();
                std::fs::write(format!("{dir}/rejected.json"), serde_json::to_string(&rejected).unwrap()).expect("probe rejected");
                std::future::pending::<()>().await;
            }''')


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=pathlib.Path)
    parser.add_argument("--runtime-only", action="store_true")
    args = parser.parse_args()
    assert args.source.resolve() != pathlib.Path(__file__).resolve().parents[1]
    instrument(args.source.resolve(), args.runtime_only)
