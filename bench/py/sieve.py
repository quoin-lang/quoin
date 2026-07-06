# Sieve of Eratosthenes - loop- and list-indexing-bound.
# Python port of bench/qn/sieve.qn. Run: python3.13 bench/py/sieve.py
#
# Kept as a class with a method to mirror the Quoin `Sieve.primesUpTo:` send.
# The boolean list is grown with append() to match the Quoin add: loop.


class Sieve:
    def primes_up_to(self, limit):
        is_prime = []
        i = 0
        while i <= limit:
            is_prime.append(True)
            i += 1

        is_prime[0] = False
        is_prime[1] = False

        p = 2
        while p * p <= limit:
            if is_prime[p]:
                i = p * p
                while i <= limit:
                    is_prime[i] = False
                    i += p
            p += 1

        primes = []
        i = 2
        while i <= limit:
            if is_prime[i]:
                primes.append(i)
            i += 1
        return primes


sieve = Sieve()
total = 0
for k in range(400):
    primes = sieve.primes_up_to(10000)
    total += len(primes) + primes[len(primes) - 1]

if total == 4480800:
    print('sieve: ok')
else:
    print('sieve: FAIL got ' + str(total))
