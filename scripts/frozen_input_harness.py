"""Step 04: owned PG, actual API binaries, fixed localhost upstream, repeatable source seed.
No credentials/configuration are read from the workspace. Never contacts business/model services.
"""

import decimal, hashlib, json, os, pathlib, shutil, socket, subprocess, tempfile, threading, time, uuid


from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


from urllib.request import Request, urlopen


from urllib.error import HTTPError

ROOT = pathlib.Path(__file__).resolve().parents[1]


PG = pathlib.Path(os.environ.get("PG_BIN", "/opt/homebrew/opt/postgresql@17/bin"))


NAMESPACE = uuid.UUID("04b431c7-4712-48d6-aa88-f273e9aa0004")


def ident(name):
    return str(uuid.uuid5(NAMESPACE, name))


def encoded(value):
    return json.dumps(
        value,
        sort_keys=True,
        ensure_ascii=False,
        separators=(",", ":"),
        default=lambda n: (
            {"exact_json_numeric_token": str(n)}
            if isinstance(n, decimal.Decimal)
            else str(n)
        ),
    )


def digest(value):
    return hashlib.sha256(encoded(value).encode()).hexdigest()


def port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def run(args, **kwargs):
    return subprocess.run([str(x) for x in args], check=True, **kwargs)


class Upstream(BaseHTTPRequestHandler):
    calls = []
    gate = threading.Event()
    mode = "hold"
    lock = threading.Lock()

    def log_message(self, *args):
        pass

    def reply(self, value, status=200):
        data = encoded(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        if self.path.startswith("/api.php/v1/user"):
            self.reply(
                {
                    "profile": {
                        "id": 1,
                        "account": "A80646",
                        "realname": "分块隔离验收",
                        "deleted": "0",
                        "view": {"projects": "11"},
                    }
                }
            )
        elif self.path.startswith("/api.php/v1/projects"):
            self.reply(
                {
                    "page": 1,
                    "limit": 100,
                    "total": 1,
                    "projects": [
                        {"id": 11, "name": "第四步合成项目", "status": "doing"}
                    ],
                }
            )
        else:
            self.reply({}, 404)

    def do_POST(self):
        data = self.rfile.read(int(self.headers.get("content-length", "0")))
        if self.path == "/api.php/v1/tokens":
            self.reply({"token": "step04-local-identity"})
            return
        if self.path != "/v1/chat/completions":
            self.reply({}, 404)
            return
        body = json.loads(data)
        prompt = body["messages"][0]["content"]
        with self.lock:
            self.calls.append(
                {
                    "at": time.monotonic(),
                    "sha256": digest(body),
                    "bytes": len(data),
                    "stage": (
                        "fields"
                        if "字段和可核查的证据：" in prompt
                        else "organize" if "接口列表:\n" in prompt else "describe"
                    ),
                }
            )
            number = len(self.calls)
        if self.mode == "hold" or (self.mode == "first" and number > 1):
            self.gate.wait(600)
        if self.mode == "fail":
            self.reply({"error": "synthetic"}, 503)
            return
        if "字段和可核查的证据：" in prompt:
            fields = json.loads(
                prompt.split("字段和可核查的证据：", 1)[1].splitlines()[0]
            )
            content = {
                "fields": [
                    {
                        "key": f["key"],
                        "name_unknown": "合成资料无名称依据",
                        "description_unknown": "合成资料不足以确认含义",
                    }
                    for f in fields
                ]
            }
        elif "接口列表:\n" in prompt:
            rows = json.loads(prompt.split("接口列表:\n", 1)[1].splitlines()[0])
            content = {
                "folders": [],
                "unplaced": [
                    {"key": r["key"], "reason": "合成资料不足以分类"} for r in rows
                ],
            }
        else:
            content = {"name": "合成冻结资料接口", "purpose": "第四步隔离验收资料"}
        self.reply(
            {
                "choices": [
                    {"message": {"content": encoded(content)}, "finish_reason": "stop"}
                ]
            }
        )


class Harness:
    def __init__(self, out, admin):
        self.out = out
        self.tmp = pathlib.Path(
            tempfile.mkdtemp(prefix="run.", dir="/tmp/nexofolio-step04")
        )
        self.api = None
        self.pg_port = port()
        self.pg_started = False
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.up = "http://127.0.0.1:" + str(self.server.server_port)
        run(
            [PG / "initdb", "-D", self.tmp / "pg", "-A", "trust", "--no-locale"],
            stdout=subprocess.DEVNULL,
        )
        run(
            [
                PG / "pg_ctl",
                "-D",
                self.tmp / "pg",
                "-l",
                self.tmp / "pg.log",
                "-o",
                f"-h 127.0.0.1 -p {self.pg_port} -k {self.tmp}",
                "start",
            ],
            stdout=subprocess.DEVNULL,
        )
        self.pg_started = True
        self.pg_pid = int((self.tmp / "pg/postmaster.pid").read_text().splitlines()[0])
        self.admin = admin

    def sql(self, db, query):
        assert db.startswith("nexofolio_step04_")
        return run(
            [
                PG / "psql",
                "-XqAt",
                "-v",
                "ON_ERROR_STOP=1",
                "-h",
                "127.0.0.1",
                "-p",
                self.pg_port,
                "-d",
                db,
            ],
            input=query,
            text=True,
            capture_output=True,
        ).stdout.strip()

    def env(self, db, api_port):
        return {
            "PATH": os.environ["PATH"],
            "DATABASE_URL": f'postgres://{os.environ["USER"]}@127.0.0.1:{self.pg_port}/{db}',
            "NEXOFOLIO_BIND_ADDR": f"127.0.0.1:{api_port}",
            "NEXOFOLIO_LOG": "info",
            "NEXOFOLIO_ZENTAO_BASE_URL": self.up + "/api.php/v1",
            "NEXOFOLIO_SESSION_KEY": "ab" * 32,
            "NEXOFOLIO_CATALOG_MODEL_BASE_URL": self.up + "/v1",
            "NEXOFOLIO_CATALOG_MODEL": "step04-fixed-localhost",
            "NEXOFOLIO_CATALOG_MODEL_API_KEY": "not-a-provider-key",
            "NEXOFOLIO_MODEL_MAX_CONCURRENCY": "1",
        }

    def create(self, db, template=None):
        args = [PG / "createdb", "-h", "127.0.0.1", "-p", self.pg_port]
        if template:
            args += ["-T", template]
        run(args + [db], stdout=subprocess.DEVNULL)
        if not template:
            with (self.out / "migration.log").open("a") as log:
                run(
                    [self.admin, "migrate"], env=self.env(db, 0), stdout=log, stderr=log
                )

    def start(self, db, binary, label):
        api_port = port()
        self.url = f"http://127.0.0.1:{api_port}"
        self.token = None
        self.pinned = self.tmp / "api-runtime"
        shutil.copy2(binary, self.pinned)
        self.active_binary_sha = hashlib.sha256(self.pinned.read_bytes()).hexdigest()
        self.log = (self.out / (label + "-api.log")).open("w")
        self.api = subprocess.Popen(
            [str(self.pinned)],
            env=self.env(db, api_port),
            stdout=self.log,
            stderr=self.log,
        )
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.api.poll() is not None:
                raise RuntimeError("API exited " + label)
            try:
                if self.request("/health/ready")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.1)
        else:
            raise TimeoutError("ready " + label)
        status, login = self.request(
            "/v1/auth/login", {"account": "A80646", "password": "fixture-pass"}
        )
        assert status == 200, (status, login)
        self.token = login["token"]
        return self.api.pid

    def request(self, path, body=None):
        headers = {"Content-Type": "application/json"}
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        if path == "/v1/collect/batches" and (body or {}).get("platform") == "nexofolio-fetcher":
            headers["X-NexoFolio-Fetcher-Version"] = "0.2.1"
        try:
            with urlopen(
                Request(
                    self.url + path,
                    data=encoded(body).encode() if body is not None else None,
                    headers=headers,
                ),
                timeout=120,
            ) as r:
                status, raw = r.status, r.read()
        except HTTPError as e:
            status, raw = e.code, e.read()
        return status, json.loads(raw) if raw else None

    def stop(self):
        if self.api:
            self.api.terminate()
            try:
                self.api.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.api.kill()
                self.api.wait()
            self.api = None
            self.log.close()
            self.pinned.unlink()
        Upstream.gate.set()
        time.sleep(0.1)
        Upstream.gate.clear()

    def cleanup(self):
        if hasattr(self, "sampler_stop"):
            self.sampler_stop.set()
        self.stop()
        Upstream.gate.set()
        self.server.shutdown()
        if self.pg_started:
            run(
                [PG / "pg_ctl", "-D", self.tmp / "pg", "stop", "-m", "fast"],
                stdout=subprocess.DEVNULL,
            )
        shutil.rmtree(self.tmp)

    def seed(self, db, count):
        from frozen_input_fixture import seed

        return seed(self, db, count)

    def measure(self, db, binary, label, project):
        from frozen_input_measurement import measure

        return measure(self, db, binary, label, project)

    def evidence(self, db, candidate):
        from frozen_input_measurement import evidence

        return evidence(self, db, candidate)

    def stats(self, db):
        from frozen_input_measurement import stats

        return stats(self, db)
