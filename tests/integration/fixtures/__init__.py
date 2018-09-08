import json
import time

import requests

session = requests.session()


class SentryLike(object):
    _healthcheck_passed = False

    @property
    def url(self):
        return "http://{}:{}".format(*self.server_address)

    def _wait(self, path):
        backoff = 0.1
        while True:
            try:
                self.get(path).raise_for_status()
                break
            except Exception as e:
                time.sleep(backoff)
                if backoff > 10:
                    raise
                backoff *= 2

    def wait_relay_healthcheck(self):
        if self._healthcheck_passed:
            return

        self._wait("/api/relay/healthcheck/")
        self._healthcheck_passed = True

    def __repr__(self):
        return "<{}({})>".format(self.__class__.__name__, repr(self.upstream))

    @property
    def dsn_public_key(self):
        return "31a5a894b4524f74a9a8d0e27e21ba91"

    @property
    def dsn(self):
        """DSN for which you will find the events in self.captured_events"""
        # bogus, we never check the DSN
        return "http://{}@{}:{}/42".format(self.dsn_public_key, *self.server_address)

    def iter_public_keys(self):
        try:
            yield self.public_key
        except AttributeError:
            pass

        if self.upstream is not None:
            if isinstance(self.upstream, tuple):
                for upstream in self.upstream:
                    yield from upstream.iter_public_keys()
            else:
                yield from self.upstream.iter_public_keys()

    def basic_project_config(self):
        return {
            "publicKeys": {self.dsn_public_key: True},
            "rev": "5ceaea8c919811e8ae7daae9fe877901",
            "disabled": False,
            "lastFetch": "2018-08-24T17:29:04.426Z",
            "lastChange": "2018-07-27T12:27:01.481Z",
            "config": {
                "allowedDomains": ["*"],
                "trustedRelays": list(self.iter_public_keys()),
                "piiConfig": {
                    "rules": {},
                    "applications": {
                        "freeform": ["@email", "@mac", "@creditcard", "@userpath"],
                        "username": ["@userpath"],
                        "ip": [],
                        "databag": [
                            "@email",
                            "@mac",
                            "@creditcard",
                            "@userpath",
                            "@password",
                        ],
                        "email": ["@email"],
                    },
                },
            },
            "slug": "python",
        }

    def send_event(self, project_id, payload=None):
        content_type = None
        if payload is None:
            payload = {"message": "Hello, World!"}

        if isinstance(payload, bytes):
            content_type = "application/octet-stream"

        if isinstance(payload, dict):
            payload = json.dumps(payload)
            content_type = "application/json"

        return self.post(
            "/api/%s/store/" % project_id,
            data=payload,
            headers={
                "Content-Type": content_type,
                "X-Sentry-Auth": (
                    "Sentry sentry_version=5, sentry_timestamp=1535376240291, "
                    "sentry_client=raven-node/2.6.3, "
                    "sentry_key={}".format(self.dsn_public_key)
                ),
            },
        )

    def request(self, method, path, **kwargs):
        assert path.startswith("/")
        return session.request(method, self.url + path, **kwargs)

    def post(self, path, **kwargs):
        return self.request("post", path, **kwargs)

    def get(self, path, **kwargs):
        return self.request("get", path, **kwargs)
