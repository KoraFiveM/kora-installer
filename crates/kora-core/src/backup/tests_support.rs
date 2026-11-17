//! A fake server folder, a fake database and a fake git for the backup tests.

use std::path::Path;

use super::dbdump::fake::FakeDb;
use super::resources::fake::FakeGit;
use super::{Host, Tools};
use crate::installer::registry::{PortsRecord, ResourceRecord, ServerEntry, ServerFile};
use crate::installer::Mode;

pub struct Rig {
    #[allow(dead_code)]
    pub tmp: tempfile::TempDir,
    pub entry: ServerEntry,
    pub db: FakeDb,
}

fn unique(prefix: &str) -> String {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    format!("{prefix}{}", hex::encode(b))
}

pub fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

impl Rig {
    pub fn new() -> Rig {
        let _ = crate::installer::registry::use_test_home();
        // encrypting with the full scrypt cost takes seconds in a debug build
        super::crypt::set_work_factor_for_tests(8);
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("srv");
        let name = unique("Srv ");
        write(&dir.join("server.cfg"), "endpoint_add_tcp \"0.0.0.0:30120\"\nexec secrets.cfg\nensure core\n");
        write(&dir.join("permissions.cfg"), "add_ace group.admin command allow\n");
        write(&dir.join("secrets.cfg"), "sv_licenseKey \"cfxk_FAKEFAKEFAKEFAKE_1234\"\nset mysql_connection_string \"mysql://kora_abc:s3cr3t-pass@127.0.0.1:3306/kora_abc?charset=utf8mb4\"\n");
        write(&dir.join("database.json"), r#"{"host":"127.0.0.1","port":3306,"database":"kora_abc","user":"kora_abc","password":"s3cr3t-pass"}"#);
        write(&dir.join("txData/default/config.json"), r#"{"global":{"serverName":"Test"}}"#);
        write(&dir.join("txData/default/data/playersDB.json"), "{}");
        write(&dir.join("txData/default/logs/admin.log"), "log line\n");
        write(&dir.join("artifacts/1234/FXServer"), "binary");
        write(&dir.join("resources/core/fxmanifest.lua"), "fx_version 'cerulean'\n");
        write(&dir.join("resources/core/.git/HEAD"), "ref: refs/heads/main\n");
        write(&dir.join("resources/local-only/fxmanifest.lua"), "-- changed by me\n");
        write(&dir.join("resources/local-only/.git/HEAD"), "ref: refs/heads/main\n");
        write(&dir.join(".kora/alerts.json"), "{}");
        write(&dir.join("kora-resource-state.json"), r#"{"version":1,"clock":0,"resources":{}}"#);
        write(&dir.join(".kora/plan-state.json"), "{}");
        let sf = ServerFile {
            name: name.clone(),
            mode: Mode::Production,
            source: Some(crate::installer::Source::Fork),
            versions: Some(crate::installer::Versions::Latest),
            dir: dir.clone(),
            ports: PortsRecord { game: 30120, txadmin: 40120 },
            game_build: 3000,
            artifact_build: 1234,
            database: Some("kora_abc".into()),
            github_owner: Some("old-owner".into()),
            created_unix: 1,
            kora_version: "0.7.0".into(),
            resources: vec![
                ResourceRecord { id: "core".into(), path: "core".into(), repository: "https://github.com/KoraFiveM/core".into(), branch: "main".into(), ..Default::default() },
                ResourceRecord { id: "local-only".into(), path: "local-only".into(), repository: "https://github.com/KoraFiveM/local-only".into(), branch: "main".into(), ..Default::default() },
            ],
        };
        sf.save().unwrap();
        let entry = ServerEntry {
            name,
            dir,
            mode: Mode::Production,
            source: Some(crate::installer::Source::Fork),
            versions: Some(crate::installer::Versions::Latest),
            ports: PortsRecord { game: 30120, txadmin: 40120 },
            created_unix: 1,
        };
        Rig { tmp, entry, db: FakeDb::default() }
    }

    /// Git answers: `core` is clean and pushed; `local-only` has changes.
    pub fn clean_git(&self) -> FakeGit {
        let g = FakeGit::default();
        g.say("rev-parse HEAD", "abcdef1234567890abcdef1234567890abcdef12");
        g.say("status --porcelain", "");
        g.say("stash list", "");
        g.say("remote get-url origin", "https://github.com/KoraFiveM/core.git");
        g.say("rev-list --count HEAD --not --remotes", "0");
        g.say("rev-parse --abbrev-ref HEAD", "main");
        g.say("local-only:status --porcelain", " M fxmanifest.lua");
        g
    }

    pub fn tools<'a>(&'a self, git: &'a FakeGit) -> Tools<'a> {
        Tools { db: &self.db, git, db_client: "mariadb".into(), dump_program: Some("mariadb-dump".into()), now: 1_700_000_000 }
    }
}

pub struct FakeHost {
    pub running_now: bool,
    pub stopped_ever: bool,
    pub fail_start: bool,
}

impl FakeHost {
    pub fn new(running: bool) -> Self {
        FakeHost { running_now: running, stopped_ever: false, fail_start: false }
    }
}

impl Host for FakeHost {
    fn running(&mut self) -> bool {
        self.running_now
    }
    fn stop(&mut self) -> Result<(), String> {
        self.running_now = false;
        self.stopped_ever = true;
        Ok(())
    }
    fn start(&mut self) -> Result<(), String> {
        if self.fail_start {
            return Err("it would not start".into());
        }
        self.running_now = true;
        Ok(())
    }
}
