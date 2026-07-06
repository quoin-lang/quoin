# Combinator pipelines - idiomatic block-based iteration; measures closure
# creation and a block call per element. Ruby port of bench/qn/combinators.qn:
# collect:/select:/sum/count:/detect:/any?: map 1:1 onto Ruby's
# map/select/sum/count/detect/any?.
# Run: `ruby bench/rb/combinators.rb`.

data = []
i = 0
while i < 1000
  data << i
  i += 1
end

total = 0
k = 0
while k < 300
  tripled = data.map { |x| (x * 3) + 1 }
  evens = tripled.select { |x| (x % 2) == 0 }
  total += evens.sum
  total += tripled.count { |x| x > 1500 }
  total += tripled.detect { |x| x > 1500 }
  total += 1 if tripled.any? { |x| x > 2900 }
  k += 1
end

if total == 225750600
  puts 'combinators: ok'
else
  puts "combinators: FAIL got #{total}"
end
