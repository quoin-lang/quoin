# Typed recursive Fibonacci - Python port of bench/qn/fib_typed.qn.
# Run: python3.13 bench/py/fib_typed.py
#
# Kept as a class with a method (not a bare function) to preserve the
# dispatch-per-call shape of the Quoin original's `.value:` send. The type
# annotations mirror the Quoin source's `|n: Integer ^Integer|`; Python does
# not act on them at runtime.


class Fib:
    def value(self, n: int) -> int:
        if n <= 1:
            return n
        return self.value(n - 1) + self.value(n - 2)


r = Fib().value(32)
if r == 2178309:
    print('fib_typed: ok')
else:
    print('fib_typed: FAIL got ' + str(r))
