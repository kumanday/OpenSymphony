"""Adversarial ACP v1 subprocess. No SDK or provider credentials needed."""
import json
import os
import subprocess
import sys
import time

mode = sys.argv[1]
session = os.environ["TEST_AUTH"] if mode == "secret_session" else "opaque/session:zero"
prompt_id = None


def send(payload):
    print(json.dumps({"jsonrpc": "2.0", **payload}), flush=True)


def respond(request, result):
    send({"id": request["id"], "result": result})


def update(text):
    send({"method": "session/update", "params": {"sessionId": session,
          "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if (mode, method) in (("setup_hang", "initialize"), ("auth_hang", "authenticate"), ("new_hang", "session/new")):
        with open("setup-waiting", "w") as marker:
            marker.write(method)
        time.sleep(60)
    if method == "initialize":
        assert message["params"]["protocolVersion"] == 1
        caps = message["params"].get("clientCapabilities", {})
        assert not caps.get("terminal")
        assert not any(caps.get("fs", {}).values())
        assert not caps.get("auth", {}).get("terminal")
        respond(message, {"protocolVersion": 2 if mode == "v2" else 1,
                          "agentInfo": {"name": os.environ.get("TEST_AUTH", "test-peer"), "version": "1"},
                          "agentCapabilities": {},
                          "authMethods": [{"id": "test_auth", "name": "Fake auth"}]})
    elif method == "authenticate":
        assert message["params"]["methodId"] == "test_auth"
        respond(message, {})
    elif method == "session/new":
        if mode == "auth_required":
            send({"id": message["id"], "error": {"code": -32000, "message": "Login required"}})
            continue
        assert message["params"]["cwd"] == os.getcwd()
        assert "CHECKOUT_SECRET" not in os.environ
        if mode == "preserve_source":
            assert os.environ["AUTH_SOURCE"] == os.environ["TEST_AUTH"]
        else:
            assert "AUTH_SOURCE" not in os.environ
        respond(message, {"sessionId": session})
    elif method == "session/prompt":
        assert message["params"]["sessionId"] == session
        prompt_id = message["id"]
        if mode == "prompt_auth_required":
            update("started")
            send({"id": prompt_id, "error": {"code": -32000, "message": "Token expired"}})
            continue
        if mode in ("missing_update", "null_update", "missing_session_id"):
            params = {"sessionId": session}
            if mode == "null_update":
                params["update"] = None
            if mode == "missing_session_id":
                params = {"update": {"sessionUpdate": "future_update"}}
            send({"method": "session/update", "params": params})
            respond(message, {"stopReason": "end_turn"})
            continue
        if mode == "evidence_flood":
            for index in range(16):
                send({"method": "_future/evidence", "params": {"index": index, "values": ["small"] * 1024}})
            respond(message, {"stopReason": "end_turn"})
            continue
        if mode == "rich_update":
            update(" ")
            update("def example():\n\treturn '  spaced  '\n" + "x" * 1024)
            respond(message, {"stopReason": "end_turn"})
            continue
        if mode == "foreign_session":
            send({"method": "session/update", "params": {"sessionId": "another-session", "update": {"sessionUpdate": "future_update"}}})
            respond(message, {"stopReason": "end_turn"})
            continue
        if mode == "eof":
            sys.exit(0)
        if mode == "crash":
            sys.exit(7)
        if mode == "malformed":
            print("{not JSON", flush=True)
            time.sleep(60)
        if mode == "oversized":
            print("x" * 2000000, flush=True)
            time.sleep(60)
        if mode == "flood":
            for _ in range(50000):
                update("flood")
            time.sleep(60)
        if mode == "stderr_flood":
            sys.stderr.write("secret" * 100000)
            sys.stderr.flush()
        if mode == "hang_tree":
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
            with open("grandchild.pid", "w") as pid_file:
                pid_file.write(str(child.pid))
        update("first")
        if mode in ("cancel", "ignore_cancel", "hang", "hang_tree"):
            continue
        if mode == "unknown_stop":
            respond(message, {"stopReason": "future_stop_reason", "_meta": {"test": True}})
            continue
        secret = os.environ.get("TEST_AUTH", "")
        if secret:
            send({"method": "_future/secret-key", "params": {secret: "redact key as well as value"}})
            update(secret)
            sys.stderr.write(secret + "\n")
            sys.stderr.flush()
        send({"method": "_future/notice", "params": {"unknown": "retained"}})
        send({"method": "session/update", "params": {"sessionId": session, "update": {"sessionUpdate": "future_update", "value": 42}}})
        send({"id": 0, "method": "_unknown/request", "params": {"value": "zero"}})
        send({"id": "opaque-request", "method": "session/request_permission", "params": {
            "sessionId": session, "toolCall": {"toolCallId": "tool-1"}, "options": [{"optionId": "opaque-allow", "name": "Allow", "kind": "allow_once"}]}})
    elif method == "session/cancel":
        assert "id" not in message
        if mode == "cancel":
            time.sleep(0.05)
            update("before cancellation response")
            send({"id": prompt_id, "result": {"stopReason": "cancelled"}})
    elif method is None:
        if message["id"] == 0:
            assert message["error"]["code"] == -32601
        elif message["id"] == "opaque-request":
            assert message["result"]["outcome"]["outcome"] == "cancelled"
            update("last")
            send({"id": prompt_id, "result": {"stopReason": "end_turn"}})
        else:
            raise AssertionError("unexpected response, possibly replied to a notification")
    else:
        raise AssertionError("unexpected client method")
