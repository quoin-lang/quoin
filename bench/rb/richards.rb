# Richards - the classic OS task-scheduler simulation (Octane port).
# Ruby port of bench/qn/richards.qn, mirroring the Quoin structure exactly:
# a small state integer (0 running, 1 runnable, 2 suspended, 3
# suspended+runnable) plus a separate held flag, linked packet lists, the
# six-task setup, and 50 schedule rounds. One megamorphic call site
# (task.run(packet)) dispatches over four task classes.
# Canonical checksums per run: queue_count = 2322, hold_count = 928.
# Run: `ruby bench/rb/richards.rb`.

ID_IDLE = 0
ID_WORKER = 1
ID_HANDLER_A = 2
ID_HANDLER_B = 3
ID_DEVICE_A = 4
ID_DEVICE_B = 5
KIND_DEVICE = 0
KIND_WORK = 1
DATA_SIZE = 4

class Packet
  attr_accessor :link, :id, :a1
  attr_reader :kind, :a2

  def initialize(link, id, kind)
    @link = link
    @id = id
    @kind = kind
    @a1 = 0
    @a2 = Array.new(DATA_SIZE, 0)
  end

  def add_to(queue)
    @link = nil
    return self if queue.nil?
    peek = queue
    peek = peek.link while peek.link
    peek.link = self
    queue
  end
end

class Tcb
  attr_reader :link, :id, :priority

  def initialize(link, id, priority, queue, task)
    @link = link
    @id = id
    @priority = priority
    @queue = queue
    @task = task
    @state = queue.nil? ? 2 : 3
    @held = false
  end

  def set_running
    @state = 0
  end

  def mark_as_not_held
    @held = false
  end

  def mark_as_held
    @held = true
  end

  def held_or_suspended?
    @held || @state == 2
  end

  def mark_as_suspended
    @state = 2 + (@state % 2)
  end

  def mark_as_runnable
    @state = @state >= 2 ? 3 : 1
  end

  def run
    packet = nil
    if @state == 3
      packet = @queue
      @queue = packet.link
      @state = @queue ? 1 : 0
    end
    @task.run(packet)
  end

  def check_priority_add(cur, packet)
    if @queue
      @queue = packet.add_to(@queue)
    else
      @queue = packet
      mark_as_runnable
      return self if @priority > cur.priority
    end
    cur
  end
end

class IdleTask
  def initialize(scheduler, v1, count)
    @scheduler = scheduler
    @v1 = v1
    @count = count
  end

  def run(_packet)
    @count -= 1
    return @scheduler.hold_current if @count == 0
    if (@v1 % 2) == 0
      @v1 = @v1 / 2
      return @scheduler.release(ID_DEVICE_A)
    end
    # The Quoin version computes (v1 / 2) ^ 0xD008 arithmetically (xorD008:)
    # only because Quoin has no bitwise operators; Ruby's native ^ is
    # bit-exact with it.
    @v1 = (@v1 / 2) ^ 0xD008
    @scheduler.release(ID_DEVICE_B)
  end
end

class DeviceTask
  def initialize(scheduler)
    @scheduler = scheduler
    @v1 = nil
  end

  def run(packet)
    if packet
      @v1 = packet
      return @scheduler.hold_current
    end
    return @scheduler.suspend_current if @v1.nil?
    v = @v1
    @v1 = nil
    @scheduler.queue(v)
  end
end

class WorkerTask
  def initialize(scheduler)
    @scheduler = scheduler
    @v1 = ID_HANDLER_A
    @v2 = 0
  end

  def run(packet)
    return @scheduler.suspend_current if packet.nil?
    @v1 = @v1 == ID_HANDLER_A ? ID_HANDLER_B : ID_HANDLER_A
    packet.id = @v1
    packet.a1 = 0
    i = 0
    while i < DATA_SIZE
      @v2 += 1
      @v2 = 1 if @v2 > 26
      packet.a2[i] = @v2
      i += 1
    end
    @scheduler.queue(packet)
  end
end

class HandlerTask
  def initialize(scheduler)
    @scheduler = scheduler
    @v1 = nil
    @v2 = nil
  end

  def run(packet)
    if packet
      if packet.kind == KIND_WORK
        @v1 = packet.add_to(@v1)
      else
        @v2 = packet.add_to(@v2)
      end
    end
    if @v1
      count = @v1.a1
      if count < DATA_SIZE
        if @v2
          v = @v2
          @v2 = @v2.link
          v.a1 = @v1.a2[count]
          @v1.a1 = count + 1
          return @scheduler.queue(v)
        end
      else
        v = @v1
        @v1 = @v1.link
        return @scheduler.queue(v)
      end
    end
    @scheduler.suspend_current
  end
end

class Scheduler
  attr_reader :queue_count, :hold_count

  def initialize
    @queue_count = 0
    @hold_count = 0
    @blocks = Array.new(6, nil)
    @list = nil
    @current_tcb = nil
    @current_id = nil
  end

  def add_task(id, priority, queue, task)
    @current_tcb = Tcb.new(@list, id, priority, queue, task)
    @list = @current_tcb
    @blocks[id] = @current_tcb
  end

  def add_running_task(id, priority, queue, task)
    add_task(id, priority, queue, task)
    @current_tcb.set_running
  end

  def add_idle_task(id, priority, queue, count)
    add_running_task(id, priority, queue, IdleTask.new(self, 1, count))
  end

  def add_worker_task(id, priority, queue)
    add_task(id, priority, queue, WorkerTask.new(self))
  end

  def add_handler_task(id, priority, queue)
    add_task(id, priority, queue, HandlerTask.new(self))
  end

  def add_device_task(id, priority, queue)
    add_task(id, priority, queue, DeviceTask.new(self))
  end

  def schedule
    @current_tcb = @list
    while @current_tcb
      if @current_tcb.held_or_suspended?
        @current_tcb = @current_tcb.link
      else
        @current_id = @current_tcb.id
        @current_tcb = @current_tcb.run
      end
    end
  end

  def release(id)
    tcb = @blocks[id]
    tcb.mark_as_not_held
    return tcb if tcb.priority > @current_tcb.priority
    @current_tcb
  end

  def hold_current
    @hold_count += 1
    @current_tcb.mark_as_held
    @current_tcb.link
  end

  def suspend_current
    @current_tcb.mark_as_suspended
    @current_tcb
  end

  def queue(packet)
    t = @blocks[packet.id]
    return nil if t.nil?
    @queue_count += 1
    packet.link = nil
    packet.id = @current_id
    t.check_priority_add(@current_tcb, packet)
  end
end

rounds = 50
q_total = 0
h_total = 0
k = 0
while k < rounds
  s = Scheduler.new

  s.add_idle_task(ID_IDLE, 0, nil, 1000)

  q = Packet.new(nil, ID_WORKER, KIND_WORK)
  q = Packet.new(q, ID_WORKER, KIND_WORK)
  s.add_worker_task(ID_WORKER, 1000, q)

  q = Packet.new(nil, ID_DEVICE_A, KIND_DEVICE)
  q = Packet.new(q, ID_DEVICE_A, KIND_DEVICE)
  q = Packet.new(q, ID_DEVICE_A, KIND_DEVICE)
  s.add_handler_task(ID_HANDLER_A, 2000, q)

  q = Packet.new(nil, ID_DEVICE_B, KIND_DEVICE)
  q = Packet.new(q, ID_DEVICE_B, KIND_DEVICE)
  q = Packet.new(q, ID_DEVICE_B, KIND_DEVICE)
  s.add_handler_task(ID_HANDLER_B, 3000, q)

  s.add_device_task(ID_DEVICE_A, 4000, nil)
  s.add_device_task(ID_DEVICE_B, 5000, nil)

  s.schedule

  q_total += s.queue_count
  h_total += s.hold_count
  k += 1
end

if q_total == rounds * 2322 && h_total == rounds * 928
  puts 'richards: ok'
else
  puts "richards: FAIL q=#{q_total} h=#{h_total}"
end
