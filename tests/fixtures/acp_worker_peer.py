"""Actual CLI ACP peer: asserts cwd, durable submission and records scoped launch inputs."""
import json
import os
import sys
import time
import uuid

profile = sys.argv[1]
session = "session-" + profile
pending = None
selected_model = None
session_mcp = []
prompt_count = 0

def send(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)

def request(method, params):
    send({"id": method, "method": method, "params": {"sessionId": session, **params}})
    response = json.loads(sys.stdin.readline())
    assert response["id"] == method and "result" in response, response
    return response["result"]

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        assert message["params"]["clientCapabilities"]["elicitation"] == {"form": {}}
        if profile == "setup_retry" and not os.path.exists(".opensymphony/setup-failed"):
            open(".opensymphony/setup-failed", "w").close()
            sys.exit(8)
        if profile == "configured":
            caps = message["params"]["clientCapabilities"]
            assert caps["fs"]["readTextFile"] and caps["fs"]["writeTextFile"] and caps["terminal"]
        send({"id": message["id"], "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": True, "mcpCapabilities": {"http": True}}}})
    elif method in ("session/new", "session/load"):
        assert message["params"]["cwd"] == os.getcwd()
        if profile == "parent_unique":
            session = "session-" + uuid.uuid4().hex if method == "session/new" else message["params"]["sessionId"]
            with open("acp-session-methods.jsonl", "a") as output:
                output.write(json.dumps({"method": method, "session": session}) + "\n")
        session_mcp = message["params"].get("mcpServers", [])
        if os.environ.get("OPENSYMPHONY_MEMORY_ENDPOINT"):
            assert len(session_mcp) == 1 and session_mcp[0]["type"] == "http"
            assert session_mcp[0]["name"] == "opensymphony-memory"
            assert session_mcp[0]["url"] == os.environ["OPENSYMPHONY_MEMORY_ENDPOINT"]
            if token := os.environ.get("OPENSYMPHONY_MEMORY_TOKEN"):
                assert session_mcp[0]["headers"] == [{"name": "Authorization", "value": "Bearer " + token}]
        if profile in ("configured", "slow_model_hang", "unprompted_retry"):
            options = [{"id": "pick", "category": "model", "name": "Model", "type": "select", "currentValue": selected_model or "profile-model", "options": [{"value": value, "name": value} for value in ("profile-model", "route-model", "other-model")]}]
            send({"id": message["id"], "result": {"sessionId": session, "configOptions": options} if method == "session/new" else {"configOptions": options}})
        else:
            send({"id": message["id"], "result": {"sessionId": session} if method == "session/new" else {}})
    elif method == "session/set_config_option":
        assert profile in ("configured", "slow_model_hang", "unprompted_retry") and message["params"]["configId"] == "pick"
        if profile == "unprompted_retry":
            if not os.path.exists(".opensymphony/first-configured"):
                open(".opensymphony/first-configured", "w").close()
            elif not os.path.exists(".opensymphony/setup-failed"):
                open(".opensymphony/setup-failed", "w").close()
                send({"id": message["id"], "error": {"code": -32602, "message": "first turn setup rejected before prompt"}})
                continue
        if profile == "slow_model_hang" and prompt_count:
            open("acp-config-waiting", "w").close()
            while not os.path.exists("acp-config-release"):
                time.sleep(0.01)
        selected_model = message["params"]["value"]
        assert selected_model in ("profile-model", "route-model", "other-model")
        options = [{"id": "pick", "category": "model", "name": "Model", "type": "select", "currentValue": selected_model, "options": [{"value": value, "name": value} for value in ("profile-model", "route-model", "other-model")]}]
        send({"id": message["id"], "result": {"configOptions": options}})
    elif method == "session/prompt":
        prompt_count += 1
        with open(".opensymphony/conversation.json") as source:
            assert json.load(source)["acp"]["status"] == "submitted"
        callback_roundtrip = False
        callback_admin_present = False
        if profile == "configured":
            assert selected_model is not None
            path = os.path.join(os.getcwd(), "acp-callback.txt")
            request("fs/write_text_file", {"path": path, "content": "configured worker"})
            assert request("fs/read_text_file", {"path": path})["content"] == "configured worker"
            terminal = request("terminal/create", {"command": sys.executable, "args": ["-c", "import os; print('configured terminal'); print('admin-present' if os.environ.get('OPENSYMPHONY_MEMORY_ADMIN_TOKEN') else 'admin-absent')"]})["terminalId"]
            assert request("terminal/wait_for_exit", {"terminalId": terminal})["exitCode"] == 0
            terminal_output = request("terminal/output", {"terminalId": terminal})["output"]
            assert "configured terminal" in terminal_output
            callback_admin_present = "admin-present" in terminal_output
            request("terminal/release", {"terminalId": terminal})
            callback_roundtrip = True
        with open("acp-worker.json", "w") as output:
            json.dump({"cwd": os.getcwd(), "profile": profile, "prompt": message["params"]["prompt"][0]["text"], "memory_project": os.environ.get("OPENSYMPHONY_MEMORY_PROJECT"), "memory_repo": os.environ.get("OPENSYMPHONY_MEMORY_EXECUTION_REPO"), "checkout_secret_present": "OPENSYMPHONY_CHECKOUT_TEST_ONLY" in os.environ, "memory_token_present": bool(os.environ.get("OPENSYMPHONY_MEMORY_TOKEN")), "memory_endpoint_present": bool(os.environ.get("OPENSYMPHONY_MEMORY_ENDPOINT")), "memory_admin_present": bool(os.environ.get("OPENSYMPHONY_MEMORY_ADMIN_TOKEN")), "memory_mcp_attached": bool(session_mcp), "selected_model": selected_model, "callback_roundtrip": callback_roundtrip, "callback_admin_present": callback_admin_present}, output)
        send({"method": "session/update", "params": {"sessionId": session, "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "ACP fixture completed"}}}})
        send({"method": "session/update", "params": {"sessionId": session, "update": {"sessionUpdate": "plan", "entries": []}}})
        send({"method": "session/update", "params": {"sessionId": session, "update": {"sessionUpdate": "usage_update", "used": 42, "size": 100}}})
        with open("acp-prompts.jsonl", "a") as output:
            output.write(json.dumps({"profile": profile, "prompt": message["params"]["prompt"][0]["text"]}) + "\n")
        if profile == "corrupt":
            os.unlink(".opensymphony/conversation.json")
            os.symlink("/dev/null", ".opensymphony/conversation.json")
            sys.exit(7)
        if profile == "crash":
            sys.exit(7)
        if profile == "permission":
            pending = message["id"]
            send({"id": "permission", "method": "session/request_permission", "params": {"sessionId": session, "toolCall": {"toolCallId": "t", "title": "Permission"}, "options": []}})
            continue
        if profile == "operator_roundtrip":
            assert request("session/request_permission", {"toolCall":{"toolCallId":"tool-1","title":"Run tests"},
                "options":[{"optionId":"allow-opaque","name":"Allow once","kind":"allow_once"},
                           {"optionId":"deny-opaque","name":"Deny once","kind":"reject_once"}]}) == \
                {"outcome":{"outcome":"selected","optionId":"allow-opaque"}}
            assert request("elicitation/create", {"mode":"form","message":"Choose region","requestedSchema":{
                "type":"object","required":["region"],"properties":{"region":{"type":"string",
                    "title":"Region?","oneOf":[{"const":"west","title":"West"}]}}}}) == \
                {"action":"accept","content":{"region":"west"}}
            with open("acp-operator-roundtrip.json", "w") as output:
                json.dump({"permission":"allow-opaque","question":"west"}, output)
        if profile in ("hang", "slow_model_hang"):
            pending = message["id"]
            continue
        stop_reason = (
            "future_stop_reason" if profile == "future_stop" else
            "vendor_error_" + os.environ["AUDIT_TOKEN"] if profile == "secret_stop" else
            "end_turn"
        )
        send({"id": message["id"], "result": {"stopReason": stop_reason, "usage": {"inputTokens": 4, "outputTokens": 2, "totalTokens": 6}}})
    elif method is None and message.get("id") == "permission":
        assert message["result"]["outcome"]["outcome"] == "cancelled"
        send({"id": pending, "result": {"stopReason": "cancelled"}})
    elif method == "session/cancel":
        send({"id": pending, "result": {"stopReason": "cancelled"}})
    else:
        raise AssertionError(method)
