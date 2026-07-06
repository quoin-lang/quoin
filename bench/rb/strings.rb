# String manipulation - concat, split, search, case mapping, join.
# Ruby port of bench/qn/strings.qn. Run: `ruby bench/rb/strings.rb`.

# The source sentence has single separator spaces and no trailing space, so
# the awk-form split(' ') matches Quoin's splitString:' ' here (19 words).
words = 'lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua'.split(' ')

checksum = 0
k = 0
while k < 15000
  sentence = ''
  words.each { |w| sentence = sentence + w + ' ' }
  checksum += sentence.length

  # sentence ends with a trailing space. Quoin's splitString:' ' keeps the
  # trailing empty field ('a b ' -> 3 parts, probed with qn -e), so use
  # split(/ /, -1) — Ruby's split(' ') is the awk form and would drop it.
  parts = sentence.split(/ /, -1)
  checksum += parts.length

  checksum += 1 if sentence.include?('tempor')
  checksum += 1 if sentence.start_with?('lorem')
  checksum += sentence.upcase.length + sentence.downcase.length
  # Quoin's index: is 0-based like Ruby's String#index (probed: both 12).
  checksum += sentence.index('dolor')
  checksum += words.join('-').length
  k += 1
end

if checksum == 7755000
  puts 'strings: ok'
else
  puts "strings: FAIL got #{checksum}"
end
