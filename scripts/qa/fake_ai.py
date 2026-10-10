"""A stand-in for an AI, for looking at Null's AI features on screen (scripts/qa/shot.sh
with NULL_QA_FAKE_AI=1): it answers as Ollama does, on this Mac only, with set answers
worked out from what it's asked. Nothing is sent anywhere, nothing is downloaded.

    python3 -I fake_ai.py PORT
"""

import json
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

NOTE = "(An answer from the stand-in AI used for screenshots.)"


def answer(system: str, user: str) -> str:
    """What a model might say to this request, made up from the request itself."""
    if "small change to part of a file" in system:
        # The selection, changed visibly: numbers grown, and a line about it.
        found = re.search(r"<selection[^>]*>\n?(.*?)</selection>", user, re.S)
        part = found.group(1) if found else ""
        changed = re.sub(r"\b(\d+)\b", lambda m: str(int(m.group(1)) * 10), part)
        indent = re.match(r"\s*", part).group(0) if part else ""
        return changed.rstrip("\n") + f"\n{indent}// checked\n"
    if "programming assistant" in system:
        return f"This line adds `a` and `b` and prints the sum. {NOTE}"
    if "commit messages" in system:
        return "Grow the numbers the sum is made of"
    if "code completion" in system:
        return "42;"
    return f"Done. {NOTE}"


def agent_step(messages):
    """Null's task agent asks with tools: list the files, read the first Rust or text one,
    write it back with its numbers grown, then say so. One step per request, worked out
    from what the earlier ones did."""
    results = [m.get("content", "") for m in messages if m.get("role") == "tool"]
    calls = [c for m in messages if m.get("role") == "assistant" for c in m.get("tool_calls") or []]

    def call(name, arguments):
        number = len(calls) + 1
        return {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {"id": f"call_{number}", "type": "function", "function": {"name": name, "arguments": json.dumps(arguments)}}
            ],
        }

    if len(results) == 0:
        return call("list_files", {})
    if len(results) == 1:
        listed = results[0].splitlines()
        files = [f for f in listed if f.endswith((".rs", ".py"))] or [f for f in listed if f.endswith((".txt", ".md"))]
        return call("read_file", {"path": files[0] if files else "README.md"})
    path = json.loads(calls[1]["function"]["arguments"]).get("path", "")
    if len(results) == 2:
        grown = re.sub(r"\b(\d+)\b", lambda m: str(int(m.group(1)) * 10), results[1])
        return call("write_file", {"path": path, "content": grown})
    return {"role": "assistant", "content": f"Grew the numbers in {path}. {NOTE}"}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def read_json(self):
        length = int(self.headers.get("Content-Length") or 0)
        try:
            return json.loads(self.rfile.read(length) or b"{}")
        except ValueError:
            return {}

    def do_GET(self):
        # The models it has, if asked.
        body = json.dumps({"models": [{"name": "fake"}], "data": [{"id": "fake"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        request = self.read_json()
        if self.path.endswith("/chat/completions") and request.get("tools"):
            body = json.dumps({"choices": [{"message": agent_step(request.get("messages") or [])}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path.endswith("/chat/completions"):
            messages = request.get("messages") or []
            system = " ".join(m.get("content", "") for m in messages if m.get("role") == "system")
            user = " ".join(m.get("content", "") for m in messages if m.get("role") == "user")
            text = answer(system, user)
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            # A few characters at a time, as a model writes ("slowly" in the request: slowly,
            # to look at it writing).
            pause = 0.5 if "slowly" in user else 0.02
            for i in range(0, len(text), 6):
                chunk = {"choices": [{"delta": {"content": text[i : i + 6]}}]}
                try:
                    self.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
                    self.wfile.flush()
                except OSError:
                    return  # Stopped listening (cancelled).
                time.sleep(pause)
            self.wfile.write(b"data: [DONE]\n\n")
            return
        if self.path.endswith("/api/generate"):
            # A fill between the code before and after the caret.
            self.send_response(200)
            self.send_header("Content-Type", "application/x-ndjson")
            self.end_headers()
            for piece in ["a + ", "b"]:
                self.wfile.write((json.dumps({"response": piece, "done": False}) + "\n").encode())
                self.wfile.flush()
                time.sleep(0.02)
            self.wfile.write((json.dumps({"response": "", "done": True}) + "\n").encode())
            return
        self.send_response(404)
        self.end_headers()


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
