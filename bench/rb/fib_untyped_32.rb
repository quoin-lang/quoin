# Untyped recursive Fibonacci at n=32 (fib_typed's workload) - in Quoin this
# row differs from fib_typed only in annotations, making the typed-vs-untyped
# gap directly readable. In Ruby every call is dynamic anyway, so this is the
# same code as fib_typed.rb; matching bench/qn/fib_untyped_32.qn.
# Run: `ruby bench/rb/fib_untyped_32.rb`.

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
  puts 'fib_untyped_32: ok'
else
  puts "fib_untyped_32: FAIL got #{r}"
end
