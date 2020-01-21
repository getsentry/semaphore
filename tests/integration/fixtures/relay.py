import json
import os
import signal
import subprocess

import pytest

from . import SentryLike

RELAY_BIN = [os.environ.get("RELAY_BIN") or "target/debug/relay"]

if os.environ.get("RELAY_AS_CARGO", "false") == "true":
    RELAY_BIN = ["cargo", "run", "--"]


class Relay(SentryLike):
    def __init__(
        self,
        server_address,
        process,
        upstream,
        public_key,
        relay_id,
        config_dir,
        options,
    ):
        self.server_address = server_address
        self.process = process
        self.upstream = upstream
        self.public_key = public_key
        self.relay_id = relay_id
        self.config_dir = config_dir
        self.options = options

    def shutdown(self, sig=signal.SIGKILL):
        self.process.send_signal(sig)

        try:
            self.process.wait(19)
        except subprocess.TimeoutExpired:
            self.process.kill()
            raise


@pytest.fixture
def relay(tmpdir, mini_sentry, request, random_port, background_process, config_dir):
    def inner(upstream, options=None, prepare=None):
        host = "127.0.0.1"
        port = random_port()

        default_opts = {
            "relay": {
                "upstream": upstream.url,
                "host": host,
                "port": port,
                "tls_port": None,
                "tls_private_key": None,
                "tls_cert": None,
            },
            "sentry": {"dsn": mini_sentry.internal_error_dsn, "enabled": True},
            "limits": {"max_api_file_upload_size": "1MiB"},
            "cache": {"batch_interval": 0},
            "logging": {"level": "trace"},
            "http": {"timeout": 2},
            "processing": {
                "enabled": False,
                "kafka_config": [],
                "topics": {
                    "events": "",
                    "attachments": "",
                    "transactions": "",
                    "outcomes": "",
                },
                "redis": "",
            },
        }

        if options is not None:
            for key in options:
                default_opts.setdefault(key, {}).update(options[key])

        dir = config_dir("relay")
        dir.join("config.yml").write(json.dumps(default_opts))

        output = subprocess.check_output(
            RELAY_BIN + ["-c", str(dir), "credentials", "generate"]
        )

        if prepare is not None:
            prepare(dir)

        process = background_process(RELAY_BIN + ["-c", str(dir), "run"])

        public_key = None
        relay_id = None

        for line in output.splitlines():
            if b"public key" in line:
                public_key = line.split()[-1].decode("ascii")
            if b"relay id" in line:
                relay_id = line.split()[-1].decode("ascii")

        assert public_key
        assert relay_id

        return Relay(
            (host, port), process, upstream, public_key, relay_id, dir, default_opts
        )

    return inner
