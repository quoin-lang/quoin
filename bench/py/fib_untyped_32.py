# Untyped recursive Fibonacci at n=32 - Python port of
# bench/qn/fib_untyped_32.qn (fib_typed's workload, untyped code: the row
# that makes Quoin's typed-vs-untyped gap directly comparable).
# Run: python3.13 bench/py/fib_untyped_32.py
#
# Kept as a class with a method (not a bare function) to preserve the
# dispatch-per-call shape of the Quoin original's `.value:` send.


class Fib:
    def value(self, n):
        if n <= 1:
            return n
        return self.value(n - 1) + self.value(n - 2)


r = Fib().value(32)
if r == 2178309:
    print('fib_untyped_32: ok')
else:
    print('fib_untyped_32: FAIL got ' + str(r))
