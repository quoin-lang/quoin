# Untyped recursive Fibonacci - in Quoin this is the dispatch-heavy path (no
# annotations, full dynamic dispatch). In Ruby every call is dynamic anyway,
# so this is the same code as fib_typed.rb at the smaller workload (n=30),
# matching bench/qn/fib_untyped.qn.
# Run: `ruby bench/rb/fib_untyped.rb`.

class Fib
  def self.value(n)
    if n <= 1
      n
    else
      value(n - 1) + value(n - 2)
    end
  end
end

r = Fib.value(30)
if r == 832040
  puts 'fib_untyped: ok'
else
  puts "fib_untyped: FAIL got #{r}"
end
