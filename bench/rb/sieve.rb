# Sieve of Eratosthenes - loop- and Array-indexing-bound.
# Ruby port of bench/qn/sieve.qn. Run: `ruby bench/rb/sieve.rb`.

class Sieve
  def self.primes_up_to(limit)
    is_prime = []
    i = 0
    while i <= limit
      is_prime << true
      i += 1
    end

    is_prime[0] = false
    is_prime[1] = false

    p = 2
    while p * p <= limit
      if is_prime[p]
        i = p * p
        while i <= limit
          is_prime[i] = false
          i += p
        end
      end
      p += 1
    end

    primes = []
    i = 2
    while i <= limit
      primes << i if is_prime[i]
      i += 1
    end
    primes
  end
end

total = 0
k = 0
while k < 400
  primes = Sieve.primes_up_to(10000)
  total += primes.length + primes[primes.length - 1]
  k += 1
end

if total == 4480800
  puts 'sieve: ok'
else
  puts "sieve: FAIL got #{total}"
end
