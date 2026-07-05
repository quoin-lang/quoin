#!/usr/bin/env python3
# A misbehaving extension: binds the socket, accepts the host connection, but NEVER
# replies to GetManifest (nor anything). Simulates a buggy/hung extension at handshake.
import socket
import sys
import time

path = sys.argv[1]
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(path)
srv.listen(1)
conn, _ = srv.accept()
# Drain nothing, reply nothing. Just sit here.
time.sleep(3600)
