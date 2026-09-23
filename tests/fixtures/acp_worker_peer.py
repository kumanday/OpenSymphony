"""Actual CLI ACP peer: asserts cwd, durable submission and records scoped launch inputs."""
import json
import os
import sys

profile = sys.argv[1]
session = "session-" + profile
pending = None
selected_model = None
session_mcp = []

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
        if profile == "setup_retry" and not os.path.exists(".opensymphony/setup-failed"):
            open(".opensymphony/setup-failed", "w").close()
            sys.exit(8)
        if profile == "configured":
            caps = message["params"]["clientCapabilities"]
            assert caps["fs"]["readTextFile"] and caps["fs"]["writeTextFile"] and caps["terminal"]
        send({"id": message["id"], "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": True, "mcpCapabilities": {"http": True}}}})
    elif method in ("session/new", "session/load"):
        assert message["params"]["cwd"] == os.getcwd()
        session_mcp = message["params"].get("mcpServers", [])
        if os.environ.get("OPENSYMPHONY_MEMORY_ENDPOINT"):
            assert len(session_mcp) == 1 and session_mcp[0]["type"] == "http"
            assert session_mcp[0]["name"] == "opensymphony-memory"
            assert session_mcp[0]["url"] == os.environ["OPENSYMPHONY_MEMORY_ENDPOINT"]
            if token := os.environ.get("OPENSYMPHONY_MEMORY_TOKEN"):
                assert session_mcp[0]["headers"] == [{"name": "Authorization", "value": "Bearer " + token}]
        if profile == "configured":
            options = [{"id": "pick", "category": "model", "name": "Model", "type": "select", "currentValue": selected_model or "profile-model", "options": [{"value": value, "name": value} for value in ("profile-model", "route-model", "other-model")]}]
            send({"id": message["id"], "result": {"sessionId": session, "configOptions": options} if method == "session/new" else {"configOptions": options}})
        else:
            send({"id": message["id"], "result": {"sessionId": session} if method == "session/new" else {}})
    elif method == "session/set_config_option":
        assert profile == "configured" and message["params"]["configId"] == "pick"
        selected_model = message["params"]["value"]
        assert selected_model in ("profile-model", "route-model", "other-model")
        options = [{"id": "pick", "category": "model", "name": "Model", "type": "select", "currentValue": selected_model, "options": [{"value": value, "name": value} for value in ("profile-model", "route-model", "other-model")]}]
        send({"id": message["id"], "result": {"configOptions": options}})
    elif method == "session/prompt":
        with open(".opensymphony/conversation.json") as source:
            assert json.load(source)["acp"]["status"] == "submitted"
        callback_roundtrip = False
        if profile == "configured":
            assert selected_model is not None
            path = os.path.join(os.getcwd(), "acp-callback.txt")
            request("fs/write_text_file", {"path": path, "content": "configured worker"})
            assert request("fs/read_text_file", {"path": path})["content"] == "configured worker"
            terminal = request("terminal/create", {"command": sys.executable, "args": ["-c", "print('configured terminal')"]})["terminalId"]
            assert request("terminal/wait_for_exit", {"terminalId": terminal})["exitCode"] == 0
            assert "configured terminal" in request("terminal/output", {"terminalId": terminal})["output"]
            request("terminal/release", {"terminalId": terminal})
            callback_roundtrip = True
        with open("acp-worker.json", "w") as output:
            json.dump({"cwd": os.getcwd(), "profile": profile, "prompt": message["params"]["prompt"][0]["text"], "memory_project": os.environ.get("OPENSYMPHONY_MEMORY_PROJECT"), "memory_repo": os.environ.get("OPENSYMPHONY_MEMORY_EXECUTION_REPO"), "checkout_secret_present": "OPENSYMPHONY_CHECKOUT_TEST_ONLY" in os.environ, "memory_token_present": bool(os.environ.get("OPENSYMPHONY_MEMORY_TOKEN")), "memory_mcp_attached": bool(session_mcp), "selected_model": selected_model, "callback_roundtrip": callback_roundtrip}, output)
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
        if profile == "hang":
            pending = message["id"]
            continue
        send({"id": message["id"], "result": {"stopReason": "end_turn", "usage": {"inputTokens": 4, "outputTokens": 2, "totalTokens": 6}}})
    elif method is None and message.get("id") == "permission":
        assert message["result"]["outcome"]["outcome"] == "cancelled"
        send({"id": pending, "result": {"stopReason": "cancelled"}})
    elif method == "session/cancel":
        send({"id": pending, "result": {"stopReason": "cancelled"}})
    else:
        raise AssertionError(method)
