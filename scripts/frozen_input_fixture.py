"""Deterministic synthetic traffic and manual knowledge for isolated step 04 tests."""

import hashlib
from frozen_input_harness import NAMESPACE, encoded, ident


def seed(h, db, count):
    status, projects = h.request("/v1/projects")
    assert status == 200
    project = projects["items"][0]["project_id"]
    now = "2026-10-13T00:00:00Z"
    for start in range(0, count, 50):
        records = []
        for i in range(start, min(start + 50, count)):
            path = (
                "/synthetic/orders"
                if i == 0
                else "/capacity/" + letters(i - 1) + "-catalogue"
            )
            body = (
                {"orderId": 101, "remark": "合成测试备注"}
                if i == 0
                else {
                    "remark": "容量测试备注 " + letters(i - 1),
                    **{
                        "field" + str(j).zfill(2): (i - 1) * 100 + j
                        for j in range(1, 50)
                    },
                }
            )
            records.append(
                {
                    "id": ident(f"record-{i}"),
                    "kind": "http_exchange",
                    "version": 1,
                    "observed_at": now,
                    "context": {
                        "client_instance_id": ident("client"),
                        "page_url": "https://synthetic.example.invalid/orders",
                        "page_title": "合成订单页面",
                        "started_at": now,
                        "completed_at": now,
                    },
                    "payload": {
                        "request": {
                            "method": "POST",
                            "url": "https://synthetic.example.invalid" + path,
                            "headers": {
                                "state": "partial",
                                "entries": [["content-type", "application/json"]],
                            },
                            "body": {
                                "state": "full",
                                "media_type": "application/json",
                                "encoding": "utf8",
                                "content": encoded(body),
                            },
                        },
                        "response": {
                            "status": 200,
                            "headers": {
                                "state": "partial",
                                "entries": [["content-type", "application/json"]],
                            },
                            "body": {
                                "state": "full",
                                "media_type": "application/json",
                                "encoding": "utf8",
                                "content": encoded(
                                    {
                                        "code": 200,
                                        "data": {
                                            "orderId": 101,
                                            "status": "created",
                                        },
                                    }
                                ),
                            },
                        },
                    },
                }
            )
        status, result = h.request(
            "/v1/collect/batches",
            {
                "batch_id": ident(f"batch-{start}"),
                "platform": "nexofolio-frozen-synthetic",
                "platform_version": "0.2.1",
                "target": {
                    "project_id": project,
                    "environment": {"name": "合成环境"},
                },
                "records": records,
            },
        )
        assert status == 200 and result["accepted"] == len(records), (
            status,
            result,
        )
    h.sql(
        db,
        """INSERT INTO knowledge.field_notes(endpoint_id,project_id,path,text,author,context)
SELECT e.id,e.project_id,'request_body:'||f.key,'合成容量测试字段 '||f.key||'；用于测量检索和字段读取成本，不代表已验证的业务语义。',jsonb_build_object('by','person','user',(SELECT id FROM access.users LIMIT 1)),jsonb_build_object('location',jsonb_build_object('in','request_body'),'field_path',f.key,'display_name','合成字段 '||f.key,'display_name_confirmation','candidate','meaning',jsonb_build_object('confirmation','candidate'),'evidence','[]'::jsonb)
FROM observe.endpoints e CROSS JOIN LATERAL (SELECT CASE WHEN n=0 THEN 'remark' ELSE 'field'||lpad(n::text,2,'0') END AS key FROM generate_series(0,49) n) f WHERE e.path_template LIKE '/capacity/%';""",
    )
    # The original special interface contributes ten explicitly synthetic manual notes.
    h.sql(
        db,
        """INSERT INTO knowledge.field_notes(endpoint_id,project_id,path,text,author,context)
SELECT e.id,e.project_id,'request_body:manual'||n,'合成历史人工说明',jsonb_build_object('by','person','user',(SELECT id FROM access.users LIMIT 1)),'{}' FROM observe.endpoints e CROSS JOIN generate_series(1,10) n WHERE e.path_template='/synthetic/orders';""",
    )
    material = h.sql(
        db,
        "SELECT jsonb_build_object('endpoints',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM observe.endpoints e),'fields',(SELECT jsonb_agg(to_jsonb(f) ORDER BY endpoint_id,path) FROM knowledge.field_notes f))",
    )
    meta = {
        "project": project,
        "interfaces": count,
        "manual_fields": (count - 1) * 50 + 10,
        "seed_sha256": hashlib.sha256(material.encode()).hexdigest(),
        "seed_namespace": str(NAMESPACE),
    }
    return meta


def letters(n):
    result = ""
    n += 1
    while n:
        n, d = divmod(n - 1, 26)
        result = chr(97 + d) + result
    return result


def rich_seed(h, db):
    _, projects = h.request("/v1/projects")
    project = projects["items"][0]["project_id"]
    now = "2026-10-13T00:00:00Z"
    for environment in ["甲环境", "乙环境"]:
        records = []
        for index, (path, body, response) in enumerate(
            [
                (
                    "/widgets/list",
                    {"filter": "active"},
                    {
                        "data": [
                            {
                                "id": 900719925474099312345,
                                "name": "长文组件",
                                "document": "完整长文语义和最后一句不可丢失。" * 4000,
                                "flag": True,
                                "nil": None,
                                "empty": [],
                                "typed": ["1", 1, False, None, {}, []],
                            }
                        ]
                    },
                ),
                (
                    "/widgets/detail",
                    {"id": 900719925474099312345, "name": "长文组件"},
                    {
                        "id": 900719925474099312345,
                        "name": "长文组件",
                        "unknownBusinessCode": "ZXQ",
                    },
                ),
                (
                    "/widgets/save",
                    {"widgetId": 900719925474099312345, "value": "仅本环境"},
                    {"status": "saved"},
                ),
            ]
        ):
            record = {
                "id": ident(environment + path),
                "kind": "http_exchange",
                "version": 1,
                "observed_at": now,
                "context": {
                    "client_instance_id": ident("rich-client"),
                    "page_url": "https://synthetic.example.invalid/rich",
                    "page_title": "跨接口合成页面",
                    "page_id": ident("rich-page"),
                    "seq": index + 1,
                    "started_at": now,
                    "completed_at": now,
                },
                "payload": {
                    "request": {
                        "method": "POST",
                        "url": "https://synthetic.example.invalid" + path,
                        "headers": {
                            "state": "partial",
                            "entries": [
                                ["content-type", "application/json"],
                                ["authorization", "Bearer synthetic-only"],
                            ],
                        },
                        "body": {
                            "state": "full",
                            "media_type": "application/json",
                            "encoding": "utf8",
                            "content": encoded(body),
                        },
                    },
                    "response": {
                        "status": 200,
                        "headers": {
                            "state": "partial",
                            "entries": [["content-type", "application/json"]],
                        },
                        "body": {
                            "state": "full",
                            "media_type": "application/json",
                            "encoding": "utf8",
                            "content": encoded(response).replace(
                                '"ZXQ"', "1.234567890123456789"
                            ),
                        },
                    },
                },
            }
            records.append(record)
        status, result = h.request(
            "/v1/collect/batches",
            {
                "batch_id": ident("rich" + environment),
                "platform": "nexofolio-frozen-synthetic",
                "platform_version": "0.2.1",
                "target": {"project_id": project, "environment": {"name": environment}},
                "records": records,
            },
        )
        assert status == 200 and result["accepted"] == 3, (status, result)
    return project
