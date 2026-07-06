# Binary trees (CLBG-style) - the allocation/GC-bound benchmark: builds and
# checks ~2^depth short-lived TreeNode objects plus one long-lived tree.
# Ruby port of bench/qn/btrees.qn. Run: `ruby bench/rb/btrees.rb`.

class TreeNode
  attr_reader :left, :right, :item

  def initialize(item, left = nil, right = nil)
    @item = item
    @left = left
    @right = right
  end

  def check
    if @left
      @item + @left.check - @right.check
    else
      @item
    end
  end
end

class Trees
  # Hand-rolled like the Quoin original (no ** in Quoin); trivial cost.
  def self.power_of_two(p)
    # Native ** (Quoin hand-rolls this only for lack of the operator; cold code).
    2**p
  end

  def self.make_tree(item, depth)
    if depth > 0
      left = make_tree(2 * item - 1, depth - 1)
      right = make_tree(2 * item, depth - 1)
      TreeNode.new(item, left, right)
    else
      TreeNode.new(item)
    end
  end

  def self.run(max_depth)
    min_depth = 4
    check_total = 0

    long_lived = make_tree(0, max_depth)

    depth = min_depth
    while depth <= max_depth
      iterations = power_of_two(max_depth - depth + min_depth)
      i = 1
      while i <= iterations
        t = make_tree(i, depth)
        check_total += t.check
        t = make_tree(0 - i, depth)
        check_total += t.check
        i += 1
      end
      depth += 2
    end
    check_total + long_lived.check
  end
end

r = Trees.run(12)
if r == -10913
  puts 'btrees: ok'
else
  puts "btrees: FAIL got #{r}"
end
