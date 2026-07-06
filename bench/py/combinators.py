# Combinator pipelines - Python port of bench/qn/combinators.qn.
# Run: python3.13 bench/py/combinators.py
#
# FIDELITY NOTE: the Quoin benchmark's declared purpose is measuring closure
# creation and per-element block invocation (collect:/select:/count:/detect:/
# any?: all derive from each:). So this port deliberately uses
# closure-per-element forms - list(map(lambda ...)), list(filter(lambda ...)),
# pred(x) invoked per element - NOT list comprehensions with inline
# expressions, which would compile the predicate/transform inline and skip the
# per-element call the benchmark exists to measure.

data = list(range(1000))

total = 0
for k in range(300):
    tripled = list(map(lambda x: (x * 3) + 1, data))
    evens = list(filter(lambda x: (x % 2) == 0, tripled))
    total += sum(evens)
    pred_count = lambda x: x > 1500
    total += sum(1 for x in tripled if pred_count(x))
    total += next(x for x in tripled if x > 1500)
    pred_any = lambda x: x > 2900
    if any(pred_any(x) for x in tripled):
        total += 1

if total == 225750600:
    print('combinators: ok')
else:
    print('combinators: FAIL got ' + str(total))
