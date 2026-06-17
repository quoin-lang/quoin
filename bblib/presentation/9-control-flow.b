true <-- {
    if: -> { |ifblock| ^^ifblock.value };
    else: -> { |_| ^^nil };
    if:else: -> { |ifblock _| ^^ifblock.value };
};

false <-- {
    if: -> { |ifblock| ^^nil };
    if:else: -> { |_ elseblock| ^^elseblock.value };
};

Block <-- {
   whileDo: -> { |block|
       s = self
       s.value.if:{
           block.value;
           ^^s.whileDo:block;
       };
   };
};

{}.class <-- {}   "* Same
