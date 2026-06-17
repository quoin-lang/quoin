Bad <- {
    doWork: -> { |n| (n > 2).if:{ 'NO'.throw } else:{ (n*10).puts } }
}

badWorker = Bad.new
(1..10).each:{ |n| badWorker.doWork:n }
