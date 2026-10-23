"""A stand-in for an OpenAI-compatible chat API (OpenRouter, OpenAI, a local server), for
examples/native_agent_e2e.sh. POST /v1/chat/completions: the first request of a conversation gets
a tool call (set_source on the set named in the system message), the next one a plain answer.
Every request body is appended to the log file. Usage: fake_chat_api.py <port> <log>"""
import json, re, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port, log = int(sys.argv[1]), sys.argv[2]


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        with open(log, 'a') as f:
            f.write(json.dumps({'auth': self.headers.get('Authorization', ''), 'body': body}) + '\n')
        msgs = body['messages']
        if msgs[-1]['role'] == 'user':
            system = msgs[0]['content']
            m = re.search(r'^Active set: (\S+)', system, re.M)
            reply = {'role': 'assistant', 'content': 'Reading the set first.', 'tool_calls': [
                {'id': 'call_1', 'type': 'function', 'function': {'name': 'set_source', 'arguments': json.dumps({'name': m.group(1) if m else 'demo'})}}]}
        else:
            reply = {'role': 'assistant', 'content': f'The set has {msgs[-1]["content"].count(chr(10))} lines. Nothing to change.'}
        out = json.dumps({'choices': [{'message': reply, 'finish_reason': 'stop'}], 'usage': {'total_tokens': 123}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(out)))
        self.end_headers()
        self.wfile.write(out)


HTTPServer(('127.0.0.1', port), H).serve_forever()
