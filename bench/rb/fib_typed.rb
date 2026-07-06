# Typed recursive Fibonacci - in Quoin this exercises the typed/devirt hot
# path (sealed Integer arithmetic, typed params and return). Ruby has no type
# annotations, so this differs from fib_untyped.rb only in workload size
# (n=32); kept as a separate file so cross-language timings stay 1:1 with
# bench/qn/fib_typed.qn.
# Run: `ruby bench/rb/fib_typed.rb`.

class Fib
  def self.value(n)
    if n <= 1
      n
    else
      value(n - 1) + value(n - 2)
    end
  end
end

r = Fib.value(32)
if r == 2178309
  puts 'fib_typed: ok'
else
  puts "fib_typed: FAIL got #{r}"
end
