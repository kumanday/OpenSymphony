"""Executable ACP callback/configuration peer; assertions are part of the test oracle."""
import json
import os
import sys
import time

mode = sys.argv[1]
session = 'services-session'
serial = 100
model_choice = "first"
reasoning = "low"
mode_choice = "ask"


def send(value):
    print(json.dumps({'jsonrpc': '2.0', **value}), flush=True)


def receive():
    line = sys.stdin.readline()
    if not line:
        raise RuntimeError('client disconnected')
    return json.loads(line)


def respond(message, result):
    send({'id': message['id'], 'result': result})


def request(method, params, expected_error=False):
    global serial
    serial += 1
    send({'id': serial, 'method': method, 'params': {'sessionId': session, **params}})
    response = receive()
    assert response['id'] == serial, response
    if expected_error:
        assert 'error' in response, response
        return response['error']
    assert 'result' in response, response
    return response['result']


def config(value='first'):
    result = [{'id': 'pick', 'name': 'Model', 'category': 'model', 'type': 'select',
             'currentValue': value, 'options': [{'group': 'models', 'name': 'Models', 'options': [
                 {'value': 'first', 'name': 'First'}, {'value': 'second', 'name': 'Second'}]}]}]
    if mode == 'config_dependent':
        result.append({'id': 'a-reasoning', 'name': 'Reasoning', 'type': 'select', 'currentValue': reasoning,
                       'options': [{'value': v, 'name': v} for v in (['low', 'high'] if value == 'second' else ['low'])]})
    if mode == 'config_dependent_mode' and value == 'second':
        result.append({'id': 'behavior', 'category': 'mode', 'name': 'Mode', 'type': 'select', 'currentValue': mode_choice, 'options': [{'value': v, 'name': v} for v in ['ask', 'code']]})
    return result


while True:
    message = receive()
    method = message.get('method')
    if method == 'initialize':
        caps = message['params']['clientCapabilities']
        enabled = not mode.startswith(('config', 'legacy')) and mode not in ('disabled', 'config', 'unsupported_config', 'legacy_mode', 'missing_mcp', 'mcp', 'mcp_argv')
        assert caps.get('terminal', False) == enabled, caps
        assert caps.get('fs', {}).get('readTextFile', False) == enabled, caps
        assert caps.get('fs', {}).get('writeTextFile', False) == enabled, caps
        assert not caps.get('auth', {}).get('terminal')
        assert not caps.get('elicitation')
        respond(message, {'protocolVersion': 1, 'agentCapabilities': {'mcpCapabilities': {'http': mode == 'mcp'}}})
    elif method == 'session/new':
        assert message['params']['cwd'] == os.getcwd()
        if mode == 'mcp':
            servers = message['params']['mcpServers']
            assert len(servers) == 1
            assert servers[0]['type'] == 'http'
            assert servers[0]['name'] == 'opensymphony-memory'
            assert servers[0]['headers'] == [{'name': 'Authorization', 'value': 'Bearer scoped-grant-610'}]
            # The peer consumes the immutable host attachment, including its scoped grant.
            import urllib.request
            req = urllib.request.Request(servers[0]['url'], headers={'Authorization': servers[0]['headers'][0]['value']})
            with urllib.request.urlopen(req) as response:
                assert response.read() == b'COE-610'
        if mode == 'mcp_argv':
            server = message['params']['mcpServers'][0]
            assert server['args'] == ['--token', 'standalone-oauth-value', '--api-key=inline-api-value']
            # Echoing a granted argument in a nonsensitive field or stderr must
            # not persist the original value in source evidence.
            print('standalone-oauth-value inline-api-value', file=sys.stderr, flush=True)
        result = {'sessionId': session}
        if mode.startswith('config') or mode == 'unsupported_config':
            result['configOptions'] = config()
        if mode.startswith('legacy'):
            result['modes'] = {'currentModeId': 'ask', 'availableModes': [{'id': 'ask', 'name': 'Ask'}, {'id': 'code', 'name': 'Code'}]}
        respond(message, result)
    elif method == 'session/set_config_option':
        if mode in ('config_auth_error', 'config_rpc_error'):
            send({'id': message['id'], 'error': {'code': -32000 if mode == 'config_auth_error' else -32603,
                 'message': 'configuration unavailable rpc-secret-610'}})
            continue
        if mode in ('config_dependent', 'config_dependent_mode'):
            if model_choice == 'first':
                assert message['params'] == {'sessionId': session, 'configId': 'pick', 'value': 'second'}
                model_choice = 'second'
            elif mode == 'config_dependent_mode':
                assert message['params'] == {'sessionId': session, 'configId': 'behavior', 'value': 'code'}
                mode_choice = 'code'
            else:
                assert message['params'] == {'sessionId': session, 'configId': 'a-reasoning', 'value': 'high'}
                reasoning = 'high'
            respond(message, {'configOptions': config(model_choice)})
        else:
            assert mode == 'config'
            assert message['params'] == {'sessionId': session, 'configId': 'pick', 'value': 'second'}
            respond(message, {'configOptions': config('second')})
    elif method == 'session/set_mode':
        assert message['params'] == {'sessionId': session, 'modeId': 'code'}
        if mode == 'legacy_rpc_error':
            send({'id': message['id'], 'error': {'code': -32603, 'message': 'mode unavailable rpc-secret-610'}})
        else:
            respond(message, {})
    elif method == 'session/prompt':
        prompt_id = message['id']
        assert not mode.endswith('_error')
        if mode == 'config_dependent_mode':
            assert model_choice == 'second' and mode_choice == 'code'
        if mode == 'config_dependent':
            assert model_choice == 'second' and reasoning == 'high'
        if mode == 'mcp_argv':
            send({'method': 'session/update', 'params': {'sessionId': session, 'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'standalone-oauth-value inline-api-value'}}}})
        elif mode in ('config', 'legacy_mode'):
            update = {'sessionUpdate': 'config_option_update', 'configOptions': config('first')} if mode == 'config' else {'sessionUpdate': 'current_mode_update', 'currentModeId': 'ask'}
            send({'method': 'session/update', 'params': {'sessionId': session, 'update': update}})
        elif mode == 'disabled':
            assert request('fs/read_text_file', {'path': os.path.join(os.getcwd(), 'file')}, True)['code'] == -32601
            assert request('terminal/create', {'command': sys.executable}, True)['code'] == -32601
        elif mode == 'response_budget':
            request('fs/read_text_file', {'path': os.path.join(os.getcwd(), 'escaped')}, True)
            assert request('fs/read_text_file', {'path': os.path.join(os.getcwd(), 'small')})['content'] == 'ok'
            terminal = request('terminal/create', {'command': sys.executable, 'args': ['-c', 'import sys; sys.stdout.write(chr(0)*2048)']})['terminalId']
            request('terminal/wait_for_exit', {'terminalId': terminal})
            output = request('terminal/output', {'terminalId': terminal})
            assert output['truncated'] and output['exitStatus']['exitCode'] == 0
            assert set(output['output']) <= {chr(0)}
            request('terminal/release', {'terminalId': terminal})
        elif mode == 'files':
            path = os.path.join(os.getcwd(), 'new', 'nested', 'file')
            text = 'uno\r\ndos 😀\nlast'
            request('fs/write_text_file', {'path': path, 'content': text})
            assert request('fs/read_text_file', {'path': path})['content'] == text
            assert request('fs/read_text_file', {'path': path, 'line': 2, 'limit': 1})['content'] == 'dos 😀\n'
            assert request('fs/read_text_file', {'path': path, 'line': 99})['content'] == ''
            assert request('fs/read_text_file', {'path': path, 'limit': 0})['content'] == ''
            for invalid in [{'line': 0}, {'line': -1}, {'line': '2'}, {'limit': -1}]:
                request('fs/read_text_file', {'path': path, **invalid}, True)
            request('fs/read_text_file', {'sessionId': 'foreign', 'path': path}, True)
            request('fs/write_text_file', {'sessionId': 'foreign', 'path': path, 'content': 'wrong'}, True)
            bad_paths = ['relative', os.path.join(os.getcwd(), '..', 'escape'), os.path.join(os.path.dirname(os.getcwd()), 'nonexistent', 'file')]
            if os.name != 'nt':
                bad_paths += [os.path.join(os.getcwd(), 'outside', 'new'), os.path.join(os.getcwd(), 'dangling')]
            for bad in bad_paths:
                request('fs/write_text_file', {'path': bad, 'content': 'escape'}, True)
            request('fs/write_text_file', {'path': path, 'content': 'x' * 1025}, True)
            request('fs/read_text_file', {'path': os.path.join(os.getcwd(), 'oversized')}, True)
            request('fs/read_text_file', {'path': os.path.join(os.getcwd(), 'invalid-utf8')}, True)
            assert request('fs/read_text_file', {'path': path})['content'] == text
        elif mode in ('terminal', 'release', 'cancel', 'shutdown', 'wait_timeout', 'wait_flood', 'owner', 'foreign_handle'):
            for params in [{'cwd': os.path.dirname(os.getcwd())}, {'env': [{'name': 'PATH', 'value': 'evil'}]}, {'env': [{'name': 'CHECKOUT_SECRET', 'value': 'stolen'}]}, {'outputByteLimit': 2049}, {'cwd': 42}, {'env': 'invalid'}]:
                request('terminal/create', {'command': sys.executable, **params}, True)
            # A late editor-like extension cannot replace host facilities or grants.
            request('_ide/attach', {'cwd': '/', 'env': {'PATH': 'evil'}, 'mcpServers': []}, True)
            if mode == 'foreign_handle':
                request('terminal/output', {'terminalId': os.environ['OTHER_TERMINAL']}, True)
                respond(message, {'stopReason': 'end_turn'})
                continue
            if mode == 'terminal':
                code = "import os,sys; assert os.environ['HOST_SCOPE']=='COE-610'; sys.stdout.buffer.write(('prefix-'+'😀'*5+'END').encode()); sys.stdout.flush(); sys.exit(7)"
                terminal = request('terminal/create', {'command': sys.executable, 'args': ['-c', code], 'outputByteLimit': 10})['terminalId']
                request('terminal/output', {'sessionId': 'foreign', 'terminalId': terminal}, True)
                assert request('terminal/wait_for_exit', {'terminalId': terminal})['exitCode'] == 7
                output = request('terminal/output', {'terminalId': terminal})
                assert output['output'] == '😀END', output
                assert output['truncated'] and output['exitStatus']['exitCode'] == 7, output
                assert request('terminal/wait_for_exit', {'terminalId': terminal})['exitCode'] == 7
                request('terminal/release', {'terminalId': terminal})
                request('terminal/output', {'terminalId': terminal}, True)
            else:
                code = "import os,time; open('terminal.pid','w').write(str(os.getpid())); time.sleep(60)"
                terminal = request('terminal/create', {'command': sys.executable, 'args': ['-c', code]})['terminalId']
                while not os.path.exists('terminal.pid'):
                    time.sleep(0.01)
                if mode == 'shutdown':
                    respond(message, {'stopReason': 'end_turn'})
                    continue
                if mode == 'owner':
                    with open('terminal.id', 'w') as marker:
                        marker.write(terminal)
                    while receive().get('method') != 'session/cancel':
                        pass
                    send({'id': prompt_id, 'result': {'stopReason': 'cancelled'}})
                    continue
                if mode == 'wait_timeout':
                    request('terminal/wait_for_exit', {'terminalId': terminal}, True)
                    request('terminal/kill', {'terminalId': terminal})
                    assert request('terminal/output', {'terminalId': terminal})['exitStatus']
                    request('terminal/release', {'terminalId': terminal})
                    respond(message, {'stopReason': 'end_turn'})
                    continue
                if mode == 'wait_flood':
                    for index in range(256):
                        send({'id': index + 1000, 'method': 'terminal/wait_for_exit', 'params': {'sessionId': session, 'terminalId': terminal}})
                    time.sleep(60)
                send({'id': 'waiting', 'method': 'terminal/wait_for_exit', 'params': {'sessionId': session, 'terminalId': terminal}})
                send({'id': 'output', 'method': 'terminal/output', 'params': {'sessionId': session, 'terminalId': terminal}})
                assert receive()['id'] == 'output'  # pending wait never blocks dispatch
                if mode == 'cancel':
                    while True:
                        response = receive()
                        if response.get('method') == 'session/cancel':
                            send({'id': prompt_id, 'result': {'stopReason': 'cancelled'}})
                            break
                    continue
                send({'id': 'release', 'method': 'terminal/release', 'params': {'sessionId': session, 'terminalId': terminal}})
                responses = [receive(), receive()]
                assert {r['id'] for r in responses} == {'waiting', 'release'}
                assert all('result' in r for r in responses), responses
                request('terminal/output', {'terminalId': terminal}, True)
        respond(message, {'stopReason': 'end_turn'})
    else:
        raise AssertionError(message)
