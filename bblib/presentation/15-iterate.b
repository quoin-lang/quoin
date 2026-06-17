#( 1 2 3 ).each:{ |n| n.puts }
(1..4).each:{ |n| n.puts }       "* Same

"* All of the below is implemented in terms of .each:

(1..4).collect:{ |n| (n*10) + n }         "* #( 11 22 33 )
(1..4).reject:{ |n| n==2 }                "* #( 1 3 )
(1..4).reduce:{ |sum n| sum+n }           "* 6
#( 2 4 6 ).all?:{ |n| n%2==0 }            "* true
(1..6).groupBy:{ |n| n%2 }                "* #{ 0: #( 2 4 ) 1: #( 1 3 5 ) }
(1..4).zip:(4..7)                         "*  #( #( 1 4 ) #( 2 5 ) #( 3 6 ) )
#( 1 2 #(3 4 #(5) ) 6 7 ).flatten         "* #( 1 2 3 4 5 6 7 )

"* And more that doesn't fit on the slide
