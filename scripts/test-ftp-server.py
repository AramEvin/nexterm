#!/usr/bin/env python3
"""FTP / FTPS test server for the integration tests (needs pyftpdlib, pyopenssl).

    python3 scripts/test-ftp-server.py <port> <root-dir> plain
    python3 scripts/test-ftp-server.py <port> <root-dir> tls <cert.pem> <key.pem>

User: ftest / Ftp-Pass-123 (test-only). Binds to 127.0.0.1 only.
"""
import logging
import sys

from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler, TLS_FTPHandler
from pyftpdlib.servers import FTPServer

port, root, mode = int(sys.argv[1]), sys.argv[2], sys.argv[3]
authorizer = DummyAuthorizer()
authorizer.add_user("ftest", "Ftp-Pass-123", root, perm="elradfmwMT")
handler = TLS_FTPHandler if mode == "tls" else FTPHandler
if mode == "tls":
    handler.certfile, handler.keyfile = sys.argv[4], sys.argv[5]
    handler.tls_control_required = True
    handler.tls_data_required = True
handler.authorizer = authorizer
base = 30000 + (port % 100) * 10
handler.passive_ports = range(base, base + 9)
logging.basicConfig(level=logging.WARNING)
FTPServer(("127.0.0.1", port), handler).serve_forever()
