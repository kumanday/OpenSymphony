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
session_mode = "plan"
verbosity = "quiet"
serial = 100
abandoned_request = None
pending_new = None


def send(payload):
    print(json.dumps({"jsonrpc": "2.0", **payload}), flush=True)


def update(text):
    send({"method": "session/update", "params": {"sessionId": session,
         "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})


def config():
    models = ('first',) if os.path.exists('remove-choice') else ('first', 'second')
    options = [{'id': 'model', 'name': 'Model', 'category': 'model', 'type': 'select',
                'currentValue': model, 'options': [{'value': v, 'name': v} for v in models]}]
    if persistence == 'config':
        options.extend([
            {'id': 'mode', 'name': 'Mode', 'category': 'mode', 'type': 'select',
             'currentValue': session_mode, 'options': [{'value': v, 'name': v} for v in ('plan', 'execute')]},
            {'id': 'verbosity', 'name': 'Verbosity', 'type': 'select',
             'currentValue': verbosity, 'options': [{'value': v, 'name': v} for v in ('quiet', 'loud')]},
        ])
    return options


def new_session(message):
    assert message['params']['cwd'] == os.getcwd()
    if services_enabled and persistence in ('load', 'resume'):
        assert message['params']['mcpServers'][0]['args'] == ['--token', 'retained-scope-grant',
                                                             '--custom', 'opaque-generic-arg']
    result = {'sessionId': session}
    if services_enabled:
        result['configOptions'] = config()
    send({'id': message['id'], 'result': result})


def callback(method, params, error=False):
    global serial
    serial += 1
    send({'id': serial, 'method': method, 'params': {'sessionId': session, **params}})
    response = json.loads(sys.stdin.readline())
    assert response['id'] == serial, response
    assert ('error' in response) == error, response
    return response.get('error') if error else response.get('result')


with open("launches", "a") as log:
    log.write(str(os.getpid()) + "\n")
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method is None and message.get('id') == abandoned_request:
        assert 'error' in message, message
        abandoned_request = None
        with open('abandoned-callback-rejected', 'w') as log:
            log.write('rejected')
        if pending_new is not None:
            new_session(pending_new)
            pending_new = None
        continue
    with open("methods", "a") as log:
        log.write(method + "\n")
    if method == "initialize":
        assert message['params']['clientCapabilities']['elicitation'] == {'form': {}}
        caps = {"loadSession": persistence == "load"}
        if persistence == "resume":
            caps["sessionCapabilities"] = {"resume": {}}
        send({"id": message["id"], "result": {"protocolVersion": 1, "agentCapabilities": caps}})
    elif method == "session/new":
        if abandoned_request is not None:
            pending_new = message
        else:
            new_session(message)
    elif method in ("session/load", "session/resume"):
        assert method == "session/" + persistence
        if os.path.exists("forget-session"):
            send({"id": message["id"], "error": {"code": -32002, "message": "session missing"}})
            if services_enabled:
                serial += 1
                abandoned_request = serial
                send({'id': serial, 'method': 'fs/write_text_file', 'params': {
                    'sessionId': message['params']['sessionId'],
                    'path': os.path.join(os.getcwd(), 'abandoned-write'), 'content': 'must reject'}})
            continue
        assert message["params"]["cwd"] == os.getcwd()
        session = message["params"]["sessionId"]
        if persistence == "load":
            update("old history")
        result = {}
        if services_enabled:
            assert message['params']['mcpServers'][0]['args'] == ['--token', 'retained-scope-grant',
                                                                 '--custom', 'opaque-generic-arg']
            result['configOptions'] = config()
        send({"id": message["id"], "result": result})
        update("live adjacent to restore response")
    elif method == 'session/set_config_option':
        assert services_enabled
        with open('.opensymphony/conversation.json') as file:
            assert json.load(file)['acp']['status'] != 'submitted'
        if mode == 'services_pre_prompt' and not os.path.exists('pre-prompt-rejected'):
            callback('fs/write_text_file', {'path': os.path.join(os.getcwd(), 'pre-prompt-write'), 'content': 'must reject'}, error=True)
            callback('terminal/create', {'command': sys.executable, 'args': ['-c', "open('pre-prompt-process','w').write('must reject')"]}, error=True)
            open('pre-prompt-rejected', 'w').close()
        if os.path.exists('stall-config'):
            with open('preparing-config', 'w') as file:
                file.write('pending')
            continue
        config_id = message['params']['configId']
        if config_id == 'model':
            model = message['params']['value']
        elif config_id == 'mode':
            session_mode = message['params']['value']
        elif config_id == 'verbosity':
            verbosity = message['params']['value']
        else:
            raise AssertionError(config_id)
        send({'id': message['id'], 'result': {'configOptions': config()}})
    elif method == "session/prompt":
        assert message["params"]["sessionId"] == session
        # The marker must already exist on disk before even the first prompt byte is observed.
        with open(".opensymphony/conversation.json") as file:
            assert json.load(file)["acp"]["status"] == "submitted"
        text = message["params"]["prompt"][0]["text"]
        if text == 'form-no-route':
            result = callback('elicitation/create', {'mode': 'form', 'message': 'Choose region',
                'requestedSchema': {'type': 'object', 'required': ['region'], 'properties': {
                    'region': {'type': 'string', 'enum': ['east', 'west']}}}})
            assert result == {'action': 'cancel'}, result
            send({'id': message['id'], 'result': {'stopReason': 'end_turn'}})
            continue
        if text == 'operator-roundtrip':
            serial = 9007199254740992
            assert callback('cursor/ask_question', {'questions': []}, error=True)['code'] == -32601
            assert callback('cursor/create_plan', {'plan': 'Ship'}, error=True)['code'] == -32601
            rejected_form = callback('elicitation/create', {'mode': 'form', 'message': 'Enter API key',
                'requestedSchema': {'type': 'object', 'required': ['key'], 'properties': {
                    'key': {'type': 'string', 'enum': ['example']}}}})
            assert rejected_form == {'action': 'cancel'}, rejected_form
            rejected_url = callback('elicitation/create', {'mode': 'url', 'message': 'Open sign-in',
                'elicitationId': 'login', 'url': 'https://example.com/sign-in'})
            assert rejected_url == {'action': 'cancel'}, rejected_url
            permission = callback('session/request_permission', {'toolCall': {'toolCallId': 'tool-1', 'title': 'Run tests'},
                'options': [{'optionId': 'allow-opaque', 'name': 'Allow once', 'kind': 'allow_once'},
                            {'optionId': 'deny-opaque', 'name': 'Deny once', 'kind': 'reject_once'}]})
            assert permission == {'outcome': {'outcome': 'selected', 'optionId': 'allow-opaque'}}, permission
            question = callback('elicitation/create', {'mode': 'form', 'message': 'Choose region', 'requestedSchema': {
                'type': 'object', 'required': ['region'], 'properties': {'region': {'type': 'string',
                    'title': 'Region?', 'oneOf': [{'const': 'east', 'title': 'East'}, {'const': 'west', 'title': 'West'}]}}}})
            assert question == {'action': 'accept', 'content': {'region': 'west'}}, question
            send({'id': message['id'], 'result': {'stopReason': 'end_turn'}})
            continue
        if text in ('operator-decline', 'operator-cancel'):
            question = callback('elicitation/create', {'mode': 'form', 'message': 'Choose region', 'requestedSchema': {
                'type': 'object', 'required': ['region'], 'properties': {'region': {'type': 'string',
                    'enum': ['east', 'west']}}}})
            assert question == {'action': 'decline' if text == 'operator-decline' else 'cancel'}, question
            send({'id': message['id'], 'result': {'stopReason': 'end_turn'}})
            continue
        if text == 'file-privacy':
            content = callback('fs/read_text_file', {'path': os.path.join(os.getcwd(), 'private-file')})['content']
            assert content == 'opaque_workspace_payload_610'
            callback('fs/write_text_file', {'path': os.path.join(os.getcwd(), 'private-copy'), 'content': content})
            terminal = callback('terminal/create', {'command': sys.executable, 'args': ['-c', "import sys; sys.stdout.write(open('private-file').read())"]})['terminalId']
            assert callback('terminal/wait_for_exit', {'terminalId': terminal})['exitCode'] == 0
            assert callback('terminal/output', {'terminalId': terminal})['output'] == content
            callback('terminal/release', {'terminalId': terminal})
        if text == 'late-callbacks':
            import time
            terminal = callback('terminal/create', {'command': sys.executable, 'args': ['-c', "import os,time; open('normal-child.pid','w').write(str(os.getpid())); time.sleep(60)"]})['terminalId']
            while not os.path.exists('normal-child.pid'):
                time.sleep(0.01)
            send({'id': message['id'], 'result': {'stopReason': 'end_turn'}})
            callback('fs/write_text_file', {'path': os.path.join(os.getcwd(), 'late-write'), 'content': 'idle mutation'}, error=True)
            callback('terminal/create', {'command': sys.executable, 'args': ['-c', "open('late-process','w').write('idle process')"]}, error=True)
            open('late-rejected', 'w').close()
            continue
        if text.startswith('config-'):
            assert (model, session_mode, verbosity) == ('second', 'execute', 'loud')
            if text != 'config-assert':
                model, session_mode, verbosity = 'first', 'plan', 'quiet'
                if text == 'config-remove':
                    open('remove-choice', 'w').close()
                if text == 'config-stall':
                    open('stall-config', 'w').close()
                send({'method': 'session/update', 'params': {'sessionId': session, 'update': {
                    'sessionUpdate': 'config_option_update', 'configOptions': config()}}})
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
        if text == "unknown-stop":
            send({"id": message["id"], "result": {"stopReason": "future_stop_reason", "usage": {"inputTokens": 3}}})
            continue
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
