# Map workload - word-frequency counting over a string-keyed Hash:
# key?/[]/[]= churn plus a keys walk.
# Ruby port of bench/qn/maps.qn. Run: `ruby bench/rb/maps.rb`.

base = 'the quick brown fox jumps over the lazy dog and the cat'
text = base
r = 0
while r < 5
  text = text + ' ' + base
  r += 1
end
# Quoin's splitString:' ' splits on every single space (no run-collapsing, no
# empty-dropping). This text has only single separator spaces and no trailing
# space, so Ruby's awk-form split(' ') yields the identical 72-word list
# (verified against `qn -e`).
words = text.split(' ')

checksum = 0
k = 0
while k < 10000
  freq = {}
  words.each do |w|
    if freq.key?(w)
      freq[w] = freq[w] + 1
    else
      freq[w] = 1
    end
  end
  checksum += freq.length + freq['the']
  freq.keys.each { |key| checksum += freq[key] }
  k += 1
end

if checksum == 1000000
  puts 'maps: ok'
else
  puts "maps: FAIL got #{checksum}"
end
