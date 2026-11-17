//! The commands that create a server's containers, as argument lists.
//!
//! Pure: nothing here runs anything. A change to what a container should look like is a change to these
//! functions and to their exact-line tests. Every container carries `kora.server=<slug>`,
//! `kora.role=game|db` and `kora.spec=<hash of the arguments>`; Kora recreates a container whose recorded hash
//! no longer matches (a different FXServer build path after an update, new ports, new limits, a newer pinned
//! image), which is safe because a container holds no state: the server's files are in the bind-mounted
//! folder and the database's in its named volume.
//!
//! Hardening, all of it asserted in the tests: no `--privileged`, no host network (unless the record says the
//! person chose it), `--cap-drop ALL` (the database image gets back only the five capabilities its entry
//! script needs to own its data folder and drop to its own user), `no-new-privileges`, an init process
//! (PID 1 would otherwise ignore SIGTERM and FXServer's teardown crash would leave zombies), the txAdmin port
//! on loopback only, and the folder as the only host path inside.

use std::path::Path;

use sha2::{Digest, Sha256};

use super::engine::EngineKind;
use super::record::ContainerRecord;
use super::{db_container_name, LABEL_ROLE, LABEL_SERVER};

/// Seconds a stop waits for a clean shutdown before the engine kills the container: the systemd unit's
/// `TimeoutStopSec` is 30 too.
pub const STOP_TIMEOUT_SECS: u32 = 30;
/// MariaDB's port inside the private network.
pub const DB_PORT: u16 = 3306;
/// `nofile` limit, like the unit's `LimitNOFILE`.
pub const NOFILE: u32 = 65535;
/// Where the database image keeps its data.
pub const DB_DATA_DIR: &str = "/var/lib/mysql";

fn s(v: impl Into<String>) -> String {
    v.into()
}

/// Can this folder be mounted at the same path inside the container? `-v src:dst` splits on `:` and a
/// control character has no business in a path; neither is escapable, so such a folder is refused with a plain reason.
pub fn check_dir(dir: &Path) -> Result<String, String> {
    let d = dir.to_str().ok_or_else(|| "the server folder's name is not valid text, so it cannot be given to a container".to_string())?;
    if !d.starts_with('/') {
        return Err("the server folder must be an absolute path to run in a container".into());
    }
    if d.contains(':') || d.chars().any(|c| c.is_control()) {
        return Err("the server folder's name contains a colon or a control character, which a container mount cannot hold; use a folder such as /srv/kora/<name>".into());
    }
    if d == "/" {
        return Err("a container must not be given the whole disk".into());
    }
    Ok(d.to_string())
}

/// The restart policy: Docker's `unless-stopped` (a stopped server stays stopped across a reboot); Podman's
/// `always`, because its boot unit (`podman-restart.service`) only restarts `always` containers. The one
/// difference: a server stopped by hand on Podman starts again after the next reboot.
pub fn restart_policy(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Docker => "unless-stopped",
        EngineKind::Podman => "always",
    }
}

/// The command FXServer is started with inside the container: what `run.sh` would `exec`, without the
/// bash script (the base image has no bash). The musl loader and library path are FXServer's own.
pub fn fx_command(build_dir: &str) -> Vec<String> {
    let b = build_dir.trim_end_matches('/');
    vec![
        format!("{b}/alpine/opt/cfx-server/ld-musl-x86_64.so.1"),
        s("--library-path"),
        format!("{b}/alpine/usr/lib/v8/:{b}/alpine/lib/:{b}/alpine/usr/lib/"),
        s("--"),
        format!("{b}/alpine/opt/cfx-server/FXServer"),
        s("+set"),
        s("citizen_dir"),
        format!("{b}/alpine/opt/cfx-server/citizen/"),
    ]
}

/// What the game container is made from.
pub struct GameSpec<'a> {
    pub slug: &'a str,
    pub dir: &'a Path,
    pub rec: &'a ContainerRecord,
    pub game_port: u16,
    pub txadmin_port: u16,
    /// The FXServer build in use (a label, so a build change is visible without reading the command).
    pub build: u32,
    pub build_dir: &'a Path,
    /// `(name, value)` pairs, not secret (`TXHOST_*`).
    pub env: Vec<(String, String)>,
}

fn hash_of(args: &[String]) -> String {
    let mut h = Sha256::new();
    for a in args {
        h.update(a.as_bytes());
        h.update([0u8]);
    }
    hex::encode(&h.finalize()[..8])
}

/// The `kora.spec` label value for these arguments (everything except the label itself).
pub fn spec_hash(args: &[String]) -> String {
    hash_of(args)
}

/// Value of the `kora.spec` label inside create arguments.
pub fn spec_label_of(args: &[String]) -> Option<String> {
    args.windows(2).find_map(|w| (w[0] == "--label").then(|| w[1].strip_prefix("kora.spec=").map(str::to_string)).flatten())
}

/// `create` for the game container. Does not start it.
pub fn game_create_args(g: &GameSpec) -> Result<Vec<String>, String> {
    let dir = check_dir(g.dir)?;
    let build = check_dir(g.build_dir)?;
    let rec = g.rec;
    let kind = rec.engine;
    let mut a: Vec<String> = vec![s("create"), s("--name"), rec.container.clone()];
    a.extend([s("--label"), format!("{LABEL_SERVER}={}", g.slug), s("--label"), format!("{LABEL_ROLE}=game"), s("--label"), format!("kora.build={}", g.build)]);
    a.extend([s("--restart"), s(restart_policy(kind)), s("--stop-timeout"), STOP_TIMEOUT_SECS.to_string(), s("--init")]);
    a.extend([s("--user"), format!("{}:{}", rec.uid, rec.gid)]);
    if kind == EngineKind::Podman && rec.rootless {
        // The person's own id is the same inside and out: files the server writes are the person's.
        a.push(s("--userns=keep-id"));
    }
    a.extend([s("--cap-drop"), s("ALL"), s("--security-opt"), s("no-new-privileges"), s("--ulimit"), format!("nofile={NOFILE}:{NOFILE}")]);
    if let Some(m) = rec.memory_mb {
        a.extend([s("--memory"), format!("{m}m")]);
    }
    if let Some(c) = rec.cpu_tenths {
        a.extend([s("--cpus"), format!("{}.{}", c / 10, c % 10)]);
    }
    if rec.read_only_root {
        a.extend([s("--read-only"), s("--tmpfs"), s("/tmp:rw,mode=1777")]);
    }
    a.extend([s("-e"), s("HOME=/tmp")]);
    for (k, v) in &g.env {
        a.extend([s("-e"), format!("{k}={v}")]);
    }
    if rec.host_network {
        a.extend([s("--network"), s("host")]);
    } else {
        if let Some(db) = &rec.db {
            a.extend([s("--network"), db.network.clone()]);
        }
        let bind = rec.bind.as_deref().map(|b| format!("{b}:")).unwrap_or_default();
        a.extend([s("-p"), format!("{bind}{p}:{p}/tcp", p = g.game_port), s("-p"), format!("{bind}{p}:{p}/udp", p = g.game_port)]);
        // txAdmin: loopback only. The panel's gateway reaches it there; remote access goes through the panel.
        a.extend([s("-p"), format!("127.0.0.1:{p}:{p}/tcp", p = g.txadmin_port)]);
    }
    a.extend([s("-v"), format!("{dir}:{dir}{}", if rec.selinux_label { ":Z" } else { "" }), s("-w"), dir]);
    // The hash covers everything above and the image and command below.
    let mut tail = vec![rec.image.clone()];
    tail.extend(fx_command(&build));
    let mut all = a.clone();
    all.extend(tail.iter().cloned());
    let h = hash_of(&all);
    // Put the label right after the name so it is easy to find; the hash was taken without it.
    a.splice(3..3, [s("--label"), format!("kora.spec={h}")]);
    a.extend(tail);
    Ok(a)
}

/// `create` for the database container. The login comes in through `env_file` (a 0600 file Kora deletes
/// right after `create`), never on the command line.
pub fn db_create_args(slug: &str, rec: &ContainerRecord, env_file: &str) -> Result<Vec<String>, String> {
    let db = rec.db.as_ref().ok_or_else(|| "this server has no database container".to_string())?;
    if env_file.contains(['\0', '\n']) {
        return Err("bad env file path".into());
    }
    let mut a: Vec<String> = vec![s("create"), s("--name"), db.container.clone()];
    a.extend([s("--label"), format!("{LABEL_SERVER}={slug}"), s("--label"), format!("{LABEL_ROLE}=db")]);
    a.extend([s("--restart"), s(restart_policy(rec.engine)), s("--stop-timeout"), STOP_TIMEOUT_SECS.to_string()]);
    a.extend([s("--network"), db.network.clone()]);
    a.extend([s("--cap-drop"), s("ALL")]);
    for cap in ["CHOWN", "DAC_OVERRIDE", "FOWNER", "SETGID", "SETUID"] {
        a.extend([s("--cap-add"), s(cap)]);
    }
    a.extend([s("--security-opt"), s("no-new-privileges")]);
    a.extend([s("--env-file"), s(env_file)]);
    a.extend([s("-v"), format!("{}:{DB_DATA_DIR}", db.volume)]);
    a.push(db.image.clone());
    Ok(a)
}

/// The environment file the database image reads on its first start (creates the database and the login;
/// the generated root password is only valid for `root@localhost` inside the container and Kora never uses it).
pub fn db_env_file(database: &str, user: &str, password: &str) -> String {
    format!("MARIADB_RANDOM_ROOT_PASSWORD=1\nMARIADB_ROOT_HOST=localhost\nMARIADB_AUTO_UPGRADE=1\nMARIADB_DATABASE={database}\nMARIADB_USER={user}\nMARIADB_PASSWORD={password}\n")
}

/// The name other containers on the private network (the game server) use to reach the database.
pub fn db_host(slug: &str) -> String {
    db_container_name(slug)
}

#[cfg(test)]
mod tests {
    use super::super::record::tests::sample;
    use super::*;

    fn game<'a>(rec: &'a ContainerRecord, dir: &'a Path, build_dir: &'a Path) -> GameSpec<'a> {
        GameSpec {
            slug: "main",
            dir,
            rec,
            game_port: 30120,
            txadmin_port: 40120,
            build: 38435,
            build_dir,
            env: vec![("TXHOST_DATA_PATH".into(), "/srv/kora/main/txData".into()), ("TXHOST_TXA_PORT".into(), "40120".into()), ("TXHOST_GAME_NAME".into(), "fivem".into())],
        }
    }

    #[test]
    fn the_game_container_command_line_is_exactly_this() {
        let rec = sample();
        let dir = Path::new("/srv/kora/main");
        let bd = Path::new("/srv/kora/main/artifacts/38435");
        let a = game_create_args(&game(&rec, dir, bd)).unwrap();
        let hash = spec_label_of(&a).expect("spec label");
        let b = "/srv/kora/main/artifacts/38435";
        let expected: Vec<String> = [
            "create",
            "--name",
            "kora-main",
            "--label",
            &format!("kora.spec={hash}"),
            "--label",
            "kora.server=main",
            "--label",
            "kora.role=game",
            "--label",
            "kora.build=38435",
            "--restart",
            "always",
            "--stop-timeout",
            "30",
            "--init",
            "--user",
            "1000:1000",
            "--userns=keep-id",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--ulimit",
            "nofile=65535:65535",
            "-e",
            "HOME=/tmp",
            "-e",
            "TXHOST_DATA_PATH=/srv/kora/main/txData",
            "-e",
            "TXHOST_TXA_PORT=40120",
            "-e",
            "TXHOST_GAME_NAME=fivem",
            "--network",
            "kora-main-net",
            "-p",
            "30120:30120/tcp",
            "-p",
            "30120:30120/udp",
            "-p",
            "127.0.0.1:40120:40120/tcp",
            "-v",
            "/srv/kora/main:/srv/kora/main",
            "-w",
            "/srv/kora/main",
            &rec.image,
            &format!("{b}/alpine/opt/cfx-server/ld-musl-x86_64.so.1"),
            "--library-path",
            &format!("{b}/alpine/usr/lib/v8/:{b}/alpine/lib/:{b}/alpine/usr/lib/"),
            "--",
            &format!("{b}/alpine/opt/cfx-server/FXServer"),
            "+set",
            "citizen_dir",
            &format!("{b}/alpine/opt/cfx-server/citizen/"),
        ]
        .iter()
        .map(|x| x.to_string())
        .collect();
        assert_eq!(a, expected);
    }

    #[test]
    fn hardening_is_always_on_and_nothing_dangerous_is_ever_asked_for() {
        let mut rec = sample();
        rec.engine = EngineKind::Docker;
        rec.rootless = false;
        rec.bind = Some("203.0.113.7".into());
        rec.memory_mb = Some(4096);
        rec.cpu_tenths = Some(25);
        rec.selinux_label = true;
        rec.read_only_root = true;
        rec.db = None;
        let dir = Path::new("/srv/kora/main");
        let a = game_create_args(&game(&rec, dir, &dir.join("artifacts/1"))).unwrap();
        let line = a.join(" ");
        for bad in
            ["--privileged", "--network host", "--pid", "--ipc", "--cap-add", "/var/run/docker.sock", "--device", "--userns=host", "--security-opt seccomp=unconfined", "--security-opt apparmor"]
        {
            assert!(!line.contains(bad), "{bad} in {line}");
        }
        assert!(line.contains("--cap-drop ALL") && line.contains("no-new-privileges") && line.contains("--init"));
        assert!(line.contains("--restart unless-stopped"), "docker's own policy");
        assert!(!line.contains("--userns"), "keep-id is rootless Podman only");
        assert!(line.contains("-p 203.0.113.7:30120:30120/tcp") && line.contains("-p 203.0.113.7:30120:30120/udp"));
        assert!(line.contains("-p 127.0.0.1:40120:40120/tcp"), "txAdmin on loopback only");
        assert!(!line.contains("-p 40120:") && !line.contains("0.0.0.0"));
        assert!(line.contains("--memory 4096m") && line.contains("--cpus 2.5"));
        assert!(line.contains("--read-only --tmpfs /tmp:rw,mode=1777"));
        assert!(line.contains("-v /srv/kora/main:/srv/kora/main:Z"), "SELinux label on the one mount");
        assert!(!line.contains("--network "), "no database container, no extra network");
        assert_eq!(a.iter().filter(|x| *x == "-v").count(), 1, "the folder is the only host path inside");
    }

    #[test]
    fn host_network_only_when_the_record_says_so_and_then_no_ports_are_published() {
        let mut rec = sample();
        rec.host_network = true;
        let dir = Path::new("/srv/kora/main");
        let line = game_create_args(&game(&rec, dir, &dir.join("artifacts/1"))).unwrap().join(" ");
        assert!(line.contains("--network host") && !line.contains(" -p "));
    }

    #[test]
    fn the_spec_label_changes_with_anything_that_matters_and_with_nothing_else() {
        let rec = sample();
        let dir = Path::new("/srv/kora/main");
        let one = game_create_args(&game(&rec, dir, &dir.join("artifacts/1"))).unwrap();
        let same = game_create_args(&game(&rec, dir, &dir.join("artifacts/1"))).unwrap();
        assert_eq!(spec_label_of(&one), spec_label_of(&same));
        let newer_build = game_create_args(&game(&rec, dir, &dir.join("artifacts/2"))).unwrap();
        assert_ne!(spec_label_of(&one), spec_label_of(&newer_build), "an update moves the build path");
        let b1 = dir.join("artifacts/1");
        let mut changed = game(&rec, dir, &b1);
        changed.game_port = 30121;
        assert_ne!(spec_label_of(&one), spec_label_of(&game_create_args(&changed).unwrap()));
        let mut other_image = rec.clone();
        other_image.image = "docker.io/library/alpine@sha256:0000000000000000000000000000000000000000000000000000000000000000".into();
        assert_ne!(spec_label_of(&one), spec_label_of(&game_create_args(&game(&other_image, dir, &dir.join("artifacts/1"))).unwrap()));
    }

    #[test]
    fn folders_a_mount_cannot_hold_are_refused_in_words() {
        assert!(check_dir(Path::new("/srv/kora/main")).is_ok());
        assert!(check_dir(Path::new("/srv/a b/main")).is_ok(), "spaces are fine: an argument, never a shell word");
        assert!(check_dir(Path::new("/srv/a:b")).unwrap_err().contains("colon"));
        assert!(check_dir(Path::new("/srv/a\nb")).is_err());
        assert!(check_dir(Path::new("relative")).is_err());
        assert!(check_dir(Path::new("/")).is_err());
        let rec = sample();
        assert!(game_create_args(&game(&rec, Path::new("/srv/x:y"), Path::new("/srv/x:y/artifacts/1"))).is_err());
    }

    #[test]
    fn the_database_container_gets_its_login_from_a_file_and_only_five_capabilities() {
        let rec = sample();
        let a = db_create_args("main", &rec, "/tmp/kora-env-123").unwrap();
        let line = a.join(" ");
        assert_eq!(
            line,
            format!(
                "create --name kora-main-db --label kora.server=main --label kora.role=db --restart always --stop-timeout 30 --network kora-main-net --cap-drop ALL --cap-add CHOWN --cap-add DAC_OVERRIDE --cap-add FOWNER --cap-add SETGID --cap-add SETUID --security-opt no-new-privileges --env-file /tmp/kora-env-123 -v kora-main-dbdata:/var/lib/mysql {}",
                rec.db.as_ref().unwrap().image
            )
        );
        assert!(!line.contains("PASSWORD") && !line.contains("-e "), "no secret and no value on the command line");
        assert!(!line.contains("-p "), "the database is never published");
        let env = db_env_file("kora_abc", "kora_abc", "pw-1");
        assert!(env.contains("MARIADB_PASSWORD=pw-1\n") && env.contains("MARIADB_ROOT_HOST=localhost"));
        let mut none = rec.clone();
        none.db = None;
        assert!(db_create_args("main", &none, "/tmp/x").is_err());
    }

    #[test]
    fn the_restart_policy_follows_the_engine() {
        assert_eq!(restart_policy(EngineKind::Docker), "unless-stopped");
        assert_eq!(restart_policy(EngineKind::Podman), "always");
    }
}
