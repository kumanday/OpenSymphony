"""Retained-session and restoration peer; records effects for host lifecycle assertions."""
import json
import os
import sys
import uuid

mode = sys.argv[1]
session = str(uuid.uuid4())
pending = None


def send(payload):
    print(json.dumps({"jsonrpc": "2.0", **payload}), flush=True)


def update(text):
    send({"method": "session/update", "params": {"sessionId": session,
         "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})


with open("launches", "a") as log:
    log.write(str(os.getpid()) + "\n")
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    with open("methods", "a") as log:
        log.write(method + "\n")
    if method == "initialize":
        caps = {"loadSession": mode == "load"}
        if mode == "resume":
            caps["sessionCapabilities"] = {"resume": {}}
        send({"id": message["id"], "result": {"protocolVersion": 1, "agentCapabilities": caps}})
    elif method == "session/new":
        assert message["params"]["cwd"] == os.getcwd()
        send({"id": message["id"], "result": {"sessionId": session}})
    elif method in ("session/load", "session/resume"):
        assert method == "session/" + mode
        if os.path.exists("forget-session"):
            send({"id": message["id"], "error": {"code": -32002, "message": "session missing"}})
            continue
        assert message["params"]["cwd"] == os.getcwd()
        session = message["params"]["sessionId"]
        if mode == "load":
            update("old history")
        send({"id": message["id"], "result": {}})
        update("live adjacent to restore response")
    elif method == "session/prompt":
        assert message["params"]["sessionId"] == session
        # The marker must already exist on disk before even the first prompt byte is observed.
        with open(".opensymphony/conversation.json") as file:
            assert json.load(file)["acp"]["status"] == "submitted"
        text = message["params"]["prompt"][0]["text"]
        if text == "crash":
            sys.exit(7)
        if text == "hang":
            pending = message["id"]
            update("pending")
            continue
        update(text)
        send({"id": message["id"], "result": {"stopReason": "end_turn"}})
    elif method == "session/cancel":
        update("cancelling")
        send({"id": pending, "result": {"stopReason": "cancelled"}})
    else:
        raise AssertionError(method)
