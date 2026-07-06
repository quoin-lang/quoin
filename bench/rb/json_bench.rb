# JSON round-trip - JSON.generate/JSON.parse over a nested array-of-hashes
# document, via Ruby's stdlib json (default compact generate).
# Ruby port of bench/qn/json.qn. Run: `ruby bench/rb/json_bench.rb`.

require 'json'

# The Quoin checksum (21640000) embeds Quoin's serializer's exact output
# length, so in general it is NOT portable across serializers. The correct
# total for Ruby's stdlib JSON.generate was computed once and frozen here; it
# happens to equal Quoin's because both emit byte-identical compact JSON for
# this document (same key order, same "0.0"/"1.5" float rendering, s.length
# = 2645, per-iteration 2645 + 50 + 10 = 2705, x 8000 = 21640000).
EXPECTED = 21640000

meta = {}
meta['name'] = 'quoin'
meta['version'] = 5

items = []
i = 0
while i < 50
  m = {}
  m['id'] = i
  m['name'] = "item-#{i}"
  m['flag'] = (i % 2) == 0
  m['score'] = i * 1.5
  items << m
  i += 1
end

doc = {}
doc['items'] = items
doc['meta'] = meta

checksum = 0
k = 0
while k < 8000
  s = JSON.generate(doc)
  parsed = JSON.parse(s)
  parsed_items = parsed['items']
  checksum += s.length + parsed_items.length + parsed_items[10]['id']
  k += 1
end

if checksum == EXPECTED
  puts 'json: ok'
else
  puts "json: FAIL got #{checksum}"
end
