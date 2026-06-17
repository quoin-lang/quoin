Multi <-- {
    y: ->  { |x  { x > 5 }| 'High: %' % x }
    y: --> { |x  { x < 5 }| 'Low: %' % x  }
    y: --> { |x { x == 5 }| 'Five: %' % x }
};

Multi.new.y:1   "* 'Low: 1'
Multi.new.y:5   "* 'Five: 5'
Multi.new.y:9   "* 'High: 9'
