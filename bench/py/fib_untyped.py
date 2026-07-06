# Untyped recursive Fibonacci - Python port of bench/qn/fib_untyped.qn.
# Run: python3.13 bench/py/fib_untyped.py
#
# Kept as a class with a method (not a bare function) to preserve the
# dispatch-per-call shape of the Quoin original's `.value:` send.


class Fib:
    def value(self, n):
        if n <= 1:
            return n
        return self.value(n - 1) + self.value(n - 2)


r = Fib().value(30)
if r == 832040:
    print('fib_untyped: ok')
else:
    print('fib_untyped: FAIL got ' + str(r))
