"""Exercise an isolated Linux Compose stack; never reuse another project's volume.

Build nexofolio-backend:foundation first. POSTGRES_IMAGE can select a reachable
registry. All generated credentials and containers are cleaned up on exit.
"""
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def run(args, *, env=None):
    return subprocess.run(args, cwd=ROOT, env=env, check=True, capture_output=True,
                          text=True, timeout=120).stdout.strip()


def status(url, expected, timeout=20):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            with HTTP.open(url, timeout=3) as response:
                last = response.status
        except urllib.error.HTTPError as error:
            last = error.code
        except OSError:
            last = None
        if last == expected:
            return
        time.sleep(0.2)
    raise AssertionError(f"{url}: expected HTTP {expected}, got {last}")


def main():
    project = "nexofolio-check-" + secrets.token_hex(4)
    password = secrets.token_urlsafe(24)
    report = {"project": project, "checks": []}
    with tempfile.TemporaryDirectory(prefix="nexofolio-check-") as directory:
        env_file = Path(directory) / "compose.env"
        env_file.write_text(
            f"POSTGRES_PASSWORD={password}\n"
            f"DATABASE_URL=postgres://nexofolio:{password}@db:5432/nexofolio\n"
            "NEXOFOLIO_HTTP_PORT=0\nNEXOFOLIO_DB_PORT=0\n"
        )
        env_file.chmod(0o600)
        compose = ["docker", "compose", "--env-file", str(env_file), "-p", project,
                   "-f", str(ROOT / "deploy/compose.yaml")]
        # Only documented image overrides are inherited; deployment credentials are ours.
        environment = os.environ.copy()
        for key in ["DATABASE_URL", "POSTGRES_PASSWORD", "NEXOFOLIO_HTTP_PORT", "NEXOFOLIO_DB_PORT"]:
            environment.pop(key, None)
        def dc(*args):
            return run(compose + list(args), env=environment)
        try:
            dc("config", "--quiet")
            dc("up", "-d", "--wait", "db")
            db_port = dc("port", "db", "5432").rsplit(":", 1)[1]
            test_env = environment | {"TEST_DATABASE_URL": f"postgres://nexofolio:{password}@127.0.0.1:{db_port}/nexofolio"}
            run(["cargo", "test", "-p", "nexofolio-backend", "--test", "postgres", "--locked", "--", "--ignored"], env=test_env)
            report["checks"].append("real_postgres_probe_and_repeatable_migrations")
            dc("up", "-d", "--wait", "api", "worker")
            url = "http://127.0.0.1:" + dc("port", "api", "8080").rsplit(":", 1)[1]
            status(url + "/health/live", 200)
            status(url + "/health/ready", 200)
            status(url + "/mcp", 401)
            report["checks"].append("linux_http_health_and_unauthorized_mcp")
            dc("run", "--rm", "migrate", "nexofolio-admin", "check-config")
            dc("run", "--rm", "migrate")
            dc("run", "--rm", "migrate")
            tables = dc("exec", "-T", "db", "psql", "-U", "nexofolio", "-d", "nexofolio", "-Atc",
                        "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename")
            assert set(tables.splitlines()) == {"_sqlx_migrations", "users", "projects", "internal_sessions", "user_project_access", "login_audit"}, tables
            report["checks"].append("container_admin_and_access_schema")
            dc("stop", "db")
            status(url + "/health/live", 200)
            status(url + "/health/ready", 503)
            dc("start", "db")
            status(url + "/health/ready", 200)
            report["checks"].append("database_outage_and_recovery")
            dc("stop", "api", "worker")
            for service in ["api", "worker"]:
                container = dc("ps", "--all", "--quiet", service)
                assert run(["docker", "inspect", "--format", "{{.State.ExitCode}}", container]) == "0"
            dc("start", "api", "worker")
            # Docker may allocate a different host port when restarting port=0 bindings.
            url = "http://127.0.0.1:" + dc("port", "api", "8080").rsplit(":", 1)[1]
            status(url + "/health/ready", 200)
            report["checks"].append("api_and_worker_graceful_stop_and_restart")
            report["runtime_libraries"] = dc("exec", "-T", "api", "ldd", "/usr/local/bin/nexofolio-api").splitlines()
            report["image"] = json.loads(run(["docker", "image", "inspect", "nexofolio-backend:foundation",
                                              "--format", '{{json .}}']))
            report["image"] = {key: report["image"][key] for key in ["Id", "Os", "Architecture"]}
            print(json.dumps(report, ensure_ascii=False, indent=2))
        finally:
            # Random project name ensures this deletes only resources created above.
            dc("down", "--volumes", "--remove-orphans")


if __name__ == "__main__":
    main()
