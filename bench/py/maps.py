# Map workload - word-frequency counting over a string-keyed dict:
# membership-test/get/set churn plus a keys walk.
# Python port of bench/qn/maps.qn. Run: python3.13 bench/py/maps.py
#
# The explicit `in`-test/get/set shape mirrors Quoin's
# containsKey?:/at:/at:put: (no Counter/defaultdict, which would change the
# map-operation mix being measured). Split semantics verified equal: neither
# text has a trailing separator, so both languages yield 72 words.

base = 'the quick brown fox jumps over the lazy dog and the cat'
text = base
for r in range(5):
    text = text + ' ' + base
words = text.split(' ')

checksum = 0
for k in range(10000):
    freq = {}
    for w in words:
        if w in freq:
            freq[w] = freq[w] + 1
        else:
            freq[w] = 1
    checksum += len(freq) + freq['the']
    for key in freq.keys():
        checksum += freq[key]

if checksum == 1000000:
    print('maps: ok')
else:
    print('maps: FAIL got ' + str(checksum))
