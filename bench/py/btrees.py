# Binary trees (CLBG-style) - the allocation/GC-bound benchmark: builds and
# checks ~2^depth short-lived TreeNode objects plus one long-lived tree.
# Python port of bench/qn/btrees.qn. Run: python3.13 bench/py/btrees.py
#
# TreeNode stays a class with a check() method to preserve the per-node
# dispatch shape of the Quoin original. Quoin's Trees.powerOfTwo: hand-rolls
# 2^p in a loop only because it is written without an exponent operator; the
# native 2 ** p is used here (cold code: called once per depth level).


class TreeNode:
    def __init__(self, item, left=None, right=None):
        self.item = item
        self.left = left
        self.right = right

    def check(self):
        if self.left is not None:
            return self.item + self.left.check() - self.right.check()
        return self.item


def make_tree(item, depth):
    if depth > 0:
        left = make_tree(2 * item - 1, depth - 1)
        right = make_tree(2 * item, depth - 1)
        return TreeNode(item, left, right)
    return TreeNode(item)


def run(max_depth):
    min_depth = 4
    check_total = 0

    long_lived = make_tree(0, max_depth)

    depth = min_depth
    while depth <= max_depth:
        iterations = 2 ** (max_depth - depth + min_depth)
        for i in range(1, iterations + 1):
            t = make_tree(i, depth)
            check_total += t.check()
            t = make_tree(-i, depth)
            check_total += t.check()
        depth += 2
    return check_total + long_lived.check()


r = run(12)
if r == -10913:
    print('btrees: ok')
else:
    print('btrees: FAIL got ' + str(r))
