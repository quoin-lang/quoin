# JSON round-trip - Python port of bench/qn/json.qn, using the stdlib json
# module with compact separators. Run: python3.13 bench/py/json_bench.py
# (named json_bench.py so it does not shadow the stdlib json module)
#
# CHECKSUM NOTE: the Quoin constant 21640000 embeds Quoin's serializer's exact
# output length, so in general it is NOT portable across serializers. The
# correct constant for Python's json.dumps(doc, separators=(',', ':')) was
# computed once and frozen below. It happens to also be 21640000: probing
# showed Quoin's JSON.generate: emits the identical compact form (no spaces,
# whole floats as "X.0", true/false lowercase), so both serializers produce a
# 2645-byte document and per-iteration 2645 + 50 + 10 = 2705; 2705 * 8000 =
# 21640000. This is a verified coincidence of formats, not a copied constant.

import json

EXPECTED = 21640000  # frozen for Python's serializer; see note above

meta = {}
meta['name'] = 'quoin'
meta['version'] = 5

items = []
for i in range(50):
    m = {}
    m['id'] = i
    m['name'] = 'item-' + str(i)
    m['flag'] = (i % 2) == 0
    m['score'] = i * 1.5
    items.append(m)

doc = {}
doc['items'] = items
doc['meta'] = meta

checksum = 0
for k in range(8000):
    s = json.dumps(doc, separators=(',', ':'))
    parsed = json.loads(s)
    parsed_items = parsed['items']
    checksum += len(s) + len(parsed_items) + parsed_items[10]['id']

if checksum == EXPECTED:
    print('json: ok')
else:
    print('json: FAIL got ' + str(checksum))
