"""One-run paid-model gate; reservations survive crashes and restarts. No secrets in evidence."""
import fcntl, hashlib, json, os, pathlib, threading, time
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from frozen_input_harness import encoded, ROOT

RUN = 'nexofolio-step11-60min-20261014'
OUT = ROOT / 'docs/audits/2026-10-14-step11-60min'
LOCK = threading.Lock()


def configuration():
    cfg = {}
    for line in (ROOT / 'deploy/.env.catalog-model').read_text().splitlines():
        if '=' in line and not line.strip().startswith('#'):
            key, value = line.split('=', 1)
            cfg[key] = value.strip().strip('\"\'')
    # Resolve only the explicitly configured model credential, never source shell files.
    key = pathlib.Path(cfg['NEXOFOLIO_CATALOG_MODEL_KEY_HOST_FILE']).expanduser().read_text().strip()
    return cfg, key


class Gate:
    def __init__(self):
        self.cfg, self.key = configuration()
        self.deadline = 0
        self.enabled = False
        self.fatal_provider_error = False
        self.full_remaining = 6
        self.metrics = OUT / 'llm-ledger.jsonl'

    def record(self, row):
        row.update(run=RUN, wall=time.time(), monotonic=time.monotonic())
        with self.metrics.open('a') as f:
            f.write(encoded(row) + '\n'); f.flush(); os.fsync(f.fileno())

    def forward(self, body, group):
        # No arbitrary options, tools, images, histories, or unbounded bodies may pass.
        assert body['model'] == self.cfg['NEXOFOLIO_CATALOG_MODEL']
        assert len(body['messages']) == 1 and body['messages'][0]['role'] == 'user'
        prompt = body['messages'][0]['content']
        assert isinstance(prompt, str) and len(prompt.encode()) <= 6000, 'input_byte_budget'
        assert set(body) <= {'model','messages','temperature','thinking','reasoning_effort','response_format','max_tokens'}
        body = dict(body, max_tokens=1024 if group.startswith('concurrency') else 4096)
        wire = encoded(body).encode(); sha = hashlib.sha256(wire).hexdigest()
        # Cross-process lock plus fsynced reservation before the network operation.
        with LOCK, (OUT / 'llm-ledger.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            prior = [json.loads(x) for x in self.metrics.read_text().splitlines()] if self.metrics.exists() else []
            number = sum(x['event'] == 'reserved' for x in prior) + 1
            if self.fatal_provider_error or not self.enabled or time.monotonic() >= self.deadline or number > 30:
                raise RuntimeError('paid_gate_closed')
            if group == 'full':
                if sum(x['event'] == 'reserved' and x['group'] == 'full' for x in prior) >= 6:
                    raise RuntimeError('full_flow_budget_closed')
            self.record(dict(event='reserved', number=number, group=group, sha256=sha,
                             input_bytes=len(prompt.encode()), input_token_budget=8192,
                             input_budget_method='conservative UTF-8 byte bound; not measured token usage',max_tokens=body['max_tokens']))
        started = time.monotonic(); metric = dict(event='result', number=number, group=group, sha256=sha)
        try:
            request = Request(self.cfg['NEXOFOLIO_CATALOG_MODEL_BASE_URL'].rstrip('/')+'/chat/completions',
                              data=wire, headers={'Content-Type':'application/json','Authorization':'Bearer '+self.key})
            # No redirect may send the key or payload to another service.
            import urllib.request
            class NoRedirect(urllib.request.HTTPRedirectHandler):
                def redirect_request(self, *args, **kwargs): return None
            with urllib.request.build_opener(NoRedirect).open(request, timeout=min(130, max(1,self.deadline-time.monotonic()))) as r:
                metric['ttfb_ms'] = (time.monotonic()-started)*1000
                raw = r.read(2*1024*1024+1); assert len(raw)<=2*1024*1024
                value = json.loads(raw); status = r.status
            metric.update(status=status, returned_model=value.get('model'), usage=value.get('usage'), finish_reason=(value.get('choices') or [{}])[0].get('finish_reason'))
            if (value.get('usage') or {}).get('prompt_tokens',0)>8192:
                metric['input_budget_exceeded']=True;self.fatal_provider_error=True;self.enabled=False
            content=(value.get('choices') or [{}])[0].get('message',{}).get('content','') or ''
            try:
                parsed=json.loads(content);metric['output_json_valid']=isinstance(parsed,dict)
                if group.startswith('concurrency'):metric['expected_output_shape']=isinstance(parsed.get('name'),str) and isinstance(parsed.get('purpose'),str)
            except (ValueError,AttributeError):metric['output_json_valid']=False
            return status, value
        except HTTPError as error:
            metric.update(status=error.code, error='provider_http_error')
            if error.code in [301,302,307,308,400,401,402,403,404,422]:
                self.fatal_provider_error=True;self.enabled=False
            return error.code, {'error':{'code':'PROVIDER_HTTP_ERROR'}}
        except Exception as error:
            metric.update(status=502, error=type(error).__name__)
            return 502, {'error':{'code':'PROVIDER_TRANSPORT_OR_BODY_ERROR'}}
        finally:
            metric['total_ms'] = (time.monotonic()-started)*1000
            with LOCK: self.record(metric)


def benchmark_bodies(model):
    return [dict(model=model, messages=[dict(role='user', content='Synthetic API documentation only. Return JSON {"name":string,"purpose":string} in at most 40 words. POST /synthetic/widget/'+str(n)+' accepts {"widgetId":101,"active":true}; returns {"ok":true}. Do not infer undocumented business rules.')],
                 temperature=0.2, thinking={'type':'enabled'}, reasoning_effort='high', response_format={'type':'json_object'}) for n in range(8)]
