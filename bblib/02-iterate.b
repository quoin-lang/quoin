KeyValuePair <- { |@key @value|
    init: -> { |key value| @key=key; @value=value }

    key -> { @key }
    value -> { @value }

    #'==:' --> { |other:KeyValuePair| (.key == other.key) && (.value == other.value) }
    #'!=:' --> { |other:KeyValuePair| (.key != other.key) || (.value != other.value) }

    #'>:'  --> { |other:KeyValuePair| (.key==other.key).if:{ .value > other.value } else:{ .key > other.key }   }
    #'>=:' --> { |other:KeyValuePair| (.key==other.key).if:{ .value >= other.value } else:{ .key >= other.key } }
    #'<:'  --> { |other:KeyValuePair| (.key==other.key).if:{ .value < other.value } else:{ .key < other.key }   }
    #'<=:' --> { |other:KeyValuePair| (.key==other.key).if:{ .value <= other.value } else:{ .key <= other.key } }

    #'~:' --> { |other:KeyValuePair| .key ~ other.key && .value ~ other.value }

    s --> { @key + ':' + @value }
};

Mixin <- Iterate <- {
     each: -> { |eachBlock:Block|
         it = self
         { it.next }.whileDefinedDo:{ |eachItem|
             eachBlock.valueWithSelfOrArg:eachItem
         };
         it.reset
     };

     collect: -> {|collectBlock:Block|
         l = #();
         .each:{ |collectItem|
             l.add:collectBlock.valueWithSelfOrArg:collectItem;
         }
         ^l
     };

     select: -> {|selectBlock:Block|
         l = #();
         .each:{ |x| (selectBlock.valueWithSelfOrArg:x).if:{ l.add:x } }
         ^l
     };

     all?: -> {|allBlock:Block|
         it = self;
         .each:{ |x|
             (allBlock.valueWithSelfOrArg:x).else:{ ^^false };
         };
         ^true
     };

     none?: -> {|block:Block|
         .each:{ |x|
             (block.valueWithSelfOrArg:x).if:{ ^^false };
         };
         ^true
     };

     any? -> { .count > 0 };

     any?: -> {|block:Block|
         .each:{ |x|
             (block.valueWithSelfOrArg:x).if:{ ^^true }
         }
         ^false
     };

     count -> {
         n = 0;
         .each:{ |_| n = n+1 };
         ^n
     }

     count: -> { |block:Block|
         n = 0;
         .each:{ |x| (block.valueWithSelfOrArg:x).if:{ n = n+1 } };
         ^n
     }

     detect: -> { |block|
         .each:{ |x| (block.valueWithSelfOrArg:x).if:{ ^^x } };
     }

     first -> { .reset; .next }
     second -> { .first; .next }
     third -> { .second; .next }
     fourth -> { .third; .next }
     fifth -> { .fourth; .next }
     last -> { .reduce:{|_ n| n } }

     flatten -> {
         l = #();
         .each:{ |x|
             (x.class==List).if:{ x.flatten.each:{ |y| l.add:y } }
                            else:{ l.add:x }
         }
         ^l
     };

     groupBy: -> { |block:Block|
         map = #{};
         .each:{ |x|
             key = block.valueWithSelfOrArg:x;

             list = map.at:key;
             list.defined?.else:{
                 list = #();
                 map.at:key put:list;
             }

             list.add:x;
         };
         ^map
     }

     contains?: -> { |item|
         .each:{ |x| (x==item).if:{ ^^true } };
         ^false
     }

     reduce: -> { |block:Block|
         sum = nil;
         .each:{ |x|
             sum.defined?.else:{ sum = (block.valueWithArgs:#(x x)).class.default };
             sum = block.valueWithArgs:#(sum x)
         }
         ^sum
     }

     reduce:into: -> { |block:Block start|
         sum = start;
         .each:{ |x| sum = block.valueWithArgs:#(sum x) }
         ^sum
     }

     sum -> {
         .sum:{|x| x }
     }

     sum: -> { |block:Block|
         sum = nil;
         .each:{ |x|
             sum.defined?.else:{ sum = (block.valueWithSelfOrArg:x).class.default };

             sum = sum + (block.valueWithSelfOrArg:x)
         }
         ^sum
     }

     max: -> { |block:Block|
         max = nil;
         .each:{ |x|
             max.defined?.if:{
                 (block.valueWithArgs:#(max x)).else:{ max = x };
             }
             else:{
                 max = x;
             }
         }
         ^max
     }

     max -> { .max:{|a b| a > b} }

     min: -> { |block:Block|
         min = nil;
         .each:{ |x|
             min.defined?.if:{
                 (block.valueWithArgs:#(min x)).if:{ min = x };
             }
             else:{
                 min = x;
             }
         }
         ^min
     }

     min -> { .min:{|a b| a > b} }

     partition: -> { |block:Block|
         trueValues = #();
         falseValues = #();

         .each:{ |x|
             ((block.valueWithSelfOrArg:x) == true).if:{ trueValues.add:x }
                                                  else:{ falseValues.add:x };
         };

         #(trueValues falseValues)
     }

     reject: -> { |block:Block|
         l = #();
         .each:{ |x| (block.valueWithSelfOrArg:x).else:{ l.add:x } }
         ^l
     }

     uniq -> {
         map = #{};
         .each:{ |x| map.at:x.id.s put:x };
         ^map.values
     }

     zip: -> { |it2|
         r = #();
         it1 = self
         val1 = it1.next
         val2 = it2.next
         { val1.defined? && val2.defined? }.whileDo:{
             r.add:#( val1 val2 );
             val1 = it1.next
             val2 = it2.next
         }
         ^r
     }

     join: -> { |sep:String|
         result = '';
         .each:{ |x|
             (result != '').if:{ result = result + sep };
             result = result + x.s
         }
         ^result
     }

     reverse -> {
         result = #();
         .each:{|x| result.push:x }
         ^result
     }

     sort: -> { |block| .list.sort:block }
     sort -> { .list.sort }

     list -> { self.select:{ true } }
};

Map <-- {
    .can:Iterate;

    .meta <-- {
        default -> { #{} }

        fromPairs: -> { |pairs| pairs.reduce:{|m kvp| m.at:kvp.key put:kvp.value } into:#{} }
    }

    can?: -> {|clz| clz==Iterate }
};

List <-- {
    .can:Iterate;

    .meta <-- {
        default -> { #() }
    }

    can?: -> {|clz| clz==Iterate }
}
