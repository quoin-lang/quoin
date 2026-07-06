# String manipulation - concat, split, search, case mapping, join.
# Python port of bench/qn/strings.qn. Run: python3.13 bench/py/strings.py
#
# Semantics verified against Quoin probes:
# - the built sentence ends with a trailing space; Quoin's splitString:' '
#   yields a trailing empty element ('a b ' -> 3 parts), exactly like
#   Python's str.split(' '), so the parts count matches (20).
# - Quoin's index: is 0-based ('lorem ipsum dolor'.index:'dolor' -> 12),
#   matching Python's str.index.
# - join: matches Python's str.join (empty elements kept).
# The per-element concatenation loop mirrors the Quoin each: block.

words = 'lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua'.split(' ')

checksum = 0
for k in range(15000):
    sentence = ''
    for w in words:
        sentence = sentence + w + ' '
    checksum += len(sentence)

    parts = sentence.split(' ')
    checksum += len(parts)

    if 'tempor' in sentence:
        checksum += 1
    if sentence.startswith('lorem'):
        checksum += 1
    checksum += len(sentence.upper()) + len(sentence.lower())
    checksum += sentence.index('dolor')
    checksum += len('-'.join(words))

if checksum == 7755000:
    print('strings: ok')
else:
    print('strings: FAIL got ' + str(checksum))
