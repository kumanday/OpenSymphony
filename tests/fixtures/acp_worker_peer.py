"""Actual CLI ACP peer: asserts cwd, durable submission and records scoped launch inputs."""
import json
import os
import sys

profile = sys.argv[1]
session = "session-" + profile
pending = None

def send(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        if profile == "setup_retry" and not os.path.exists(".opensymphony/setup-failed"):
            open(".opensymphony/setup-failed", "w").close()
            sys.exit(8)
        send({"id": message["id"], "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": True, "mcpCapabilities": {"http": True}}}})
    elif method in ("session/new", "session/load"):
        assert message["params"]["cwd"] == os.getcwd()
        send({"id": message["id"], "result": {"sessionId": session} if method == "session/new" else {}})
    elif method == "session/prompt":
        with open(".opensymphony/conversation.json") as source:
            assert json.load(source)["acp"]["status"] == "submitted"
        with open("acp-worker.json", "w") as output:
            json.dump({"cwd": os.getcwd(), "profile": profile, "prompt": message["params"]["prompt"][0]["text"], "memory_project": os.environ.get("OPENSYMPHONY_MEMORY_PROJECT"), "memory_repo": os.environ.get("OPENSYMPHONY_MEMORY_EXECUTION_REPO"), "checkout_secret_present": "OPENSYMPHONY_CHECKOUT_TEST_ONLY" in os.environ, "memory_token_present": bool(os.environ.get("OPENSYMPHONY_MEMORY_TOKEN"))}, output)
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
