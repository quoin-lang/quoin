{ Error.throw }.catch:{ |e:Error| e == Error "(Class)" };
{ 42.throw    }.catch:{ |n|       n == 42              };

Error <- CustomError <- {}
{ CustomError.new.throw }.catch:{ |ex:CustomError| ex ~ CustomError };

{ CustomError.new.throw }
    .catch:  { |ex:CustomError| "Logic for CustomError"     }
    .catch:  { |ex:Error| "Logic for all other error types" }
    .finally:{ "Always execute this block"                  };
