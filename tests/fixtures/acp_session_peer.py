"""Retained-session and restoration peer; records effects for host lifecycle assertions."""
import json
import os
import sys
import uuid

mode = sys.argv[1]
services_enabled = mode.startswith("services")
persistence = mode.removeprefix("services_")
session = str(uuid.uuid4())
pending = None
terminal = None
model = "first"
serial = 100


def send(payload):
    print(json.dumps({"jsonrpc": "2.0", **payload}), flush=True)


def update(text):
    send({"method": "session/update", "params": {"sessionId": session,
         "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})


def config():
    return [{'id': 'model', 'name': 'Model', 'category': 'model', 'type': 'select',
             'currentValue': model, 'options': [{'value': v, 'name': v} for v in ('first', 'second')]}]


def callback(method, params, error=False):
    global serial
    serial += 1
    send({'id': serial, 'method': method, 'params': {'sessionId': session, **params}})
    response = json.loads(sys.stdin.readline())
    assert response['id'] == serial, response
    assert ('error' in response) == error, response
    return response.get('result')


with open("launches", "a") as log:
    log.write(str(os.getpid()) + "\n")
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    with open("methods", "a") as log:
        log.write(method + "\n")
    if method == "initialize":
        caps = {"loadSession": persistence == "load"}
        if persistence == "resume":
            caps["sessionCapabilities"] = {"resume": {}}
        send({"id": message["id"], "result": {"protocolVersion": 1, "agentCapabilities": caps}})
    elif method == "session/new":
        assert message["params"]["cwd"] == os.getcwd()
        if services_enabled and persistence in ('load', 'resume'):
            assert message['params']['mcpServers'][0]['args'] == ['--token', 'retained-scope-grant']
        result = {"sessionId": session}
        if services_enabled:
            result['configOptions'] = config()
        send({"id": message["id"], "result": result})
    elif method in ("session/load", "session/resume"):
        assert method == "session/" + persistence
        if os.path.exists("forget-session"):
            send({"id": message["id"], "error": {"code": -32002, "message": "session missing"}})
            continue
        assert message["params"]["cwd"] == os.getcwd()
        session = message["params"]["sessionId"]
        if persistence == "load":
            update("old history")
        result = {}
        if services_enabled:
            assert message['params']['mcpServers'][0]['args'] == ['--token', 'retained-scope-grant']
            result['configOptions'] = config()
        send({"id": message["id"], "result": result})
        update("live adjacent to restore response")
    elif method == 'session/set_config_option':
        assert services_enabled
        assert message['params']['configId'] == 'model'
        model = message['params']['value']
        send({'id': message['id'], 'result': {'configOptions': config()}})
    elif method == "session/prompt":
        assert message["params"]["sessionId"] == session
        # The marker must already exist on disk before even the first prompt byte is observed.
        with open(".opensymphony/conversation.json") as file:
            assert json.load(file)["acp"]["status"] == "submitted"
        text = message["params"]["prompt"][0]["text"]
        if text == 'services-cancel':
            import time
            terminal = callback('terminal/create', {'command': sys.executable, 'args': ['-c', "import os,time; open('callback.pid','w').write(str(os.getpid())); time.sleep(60)"]})['terminalId']
            while not os.path.exists('callback.pid'):
                time.sleep(0.01)
            pending = message['id']
            update('services-active')
            continue
        if text == 'services-next':
            callback('terminal/output', {'terminalId': terminal}, error=True)
            path = os.path.join(os.getcwd(), 'second-turn-file')
            callback('fs/write_text_file', {'path': path, 'content': 'fresh epoch'})
            assert callback('fs/read_text_file', {'path': path})['content'] == 'fresh epoch'
            terminal = callback('terminal/create', {'command': sys.executable, 'args': ['-c', "print('fresh process')"]})['terminalId']
            assert callback('terminal/wait_for_exit', {'terminalId': terminal})['exitCode'] == 0
            callback('terminal/release', {'terminalId': terminal})
            model = 'first'
            send({'method': 'session/update', 'params': {'sessionId': session, 'update': {'sessionUpdate': 'config_option_update', 'configOptions': config()}}})
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
