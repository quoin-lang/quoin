# Richards - the classic OS task-scheduler simulation (a port of the Octane
# benchmark, mirroring the structure of bench/qn/richards.qn).
# Run: python3.13 bench/py/richards.py
#
# Structure kept bit-exact with the Quoin version:
# - TCB state is a small state integer (0 running, 1 runnable, 2 suspended,
#   3 suspended+runnable) plus a separate held flag, as in the Quoin port.
# - Linked packet lists, the six-task setup, 50 scheduler rounds.
# - The one Quoin-specific workaround is NOT mirrored: Quoin lacks bitwise
#   operators so its idle task computes `(v1 >> 1) ^ 0xD008` arithmetically
#   (xorD008:); Python has native bitwise ops, so this port uses
#   `(v1 // 2) ^ 0xD008` directly (same value, the canonical Richards form).
# Canonical checksums: queue_count = 2322 and hold_count = 928 per run.

ID_IDLE = 0
ID_WORKER = 1
ID_HANDLER_A = 2
ID_HANDLER_B = 3
ID_DEVICE_A = 4
ID_DEVICE_B = 5
KIND_DEVICE = 0
KIND_WORK = 1
DATA_SIZE = 4


class Packet:
    def __init__(self, link, id, kind):
        self.link = link
        self.id = id
        self.kind = kind
        self.a1 = 0
        self.a2 = [0] * DATA_SIZE

    def add_to(self, queue):
        self.link = None
        if queue is None:
            return self
        peek = queue
        while peek.link is not None:
            peek = peek.link
        peek.link = self
        return queue


class Tcb:
    def __init__(self, link, id, priority, queue, task):
        self.link = link
        self.id = id
        self.priority = priority
        self.queue = queue
        self.task = task
        self.state = 3 if queue is not None else 2
        self.held = False

    def set_running(self):
        self.state = 0

    def mark_as_not_held(self):
        self.held = False

    def mark_as_held(self):
        self.held = True

    def is_held_or_suspended(self):
        return self.held or self.state == 2

    def mark_as_suspended(self):
        self.state = 2 + (self.state % 2)

    def mark_as_runnable(self):
        if self.state >= 2:
            self.state = 3
        else:
            self.state = 1

    def run(self):
        packet = None
        if self.state == 3:
            packet = self.queue
            self.queue = packet.link
            if self.queue is not None:
                self.state = 1
            else:
                self.state = 0
        return self.task.run(packet)

    def check_priority_add(self, cur, packet):
        if self.queue is not None:
            self.queue = packet.add_to(self.queue)
        else:
            self.queue = packet
            self.mark_as_runnable()
            if self.priority > cur.priority:
                return self
        return cur


class IdleTask:
    def __init__(self, scheduler, v1, count):
        self.scheduler = scheduler
        self.v1 = v1
        self.count = count

    def run(self, packet):
        self.count -= 1
        if self.count == 0:
            return self.scheduler.hold_current()
        if (self.v1 % 2) == 0:
            self.v1 = self.v1 // 2
            return self.scheduler.release(ID_DEVICE_A)
        self.v1 = (self.v1 // 2) ^ 0xD008
        return self.scheduler.release(ID_DEVICE_B)


class DeviceTask:
    def __init__(self, scheduler):
        self.scheduler = scheduler
        self.v1 = None

    def run(self, packet):
        if packet is not None:
            self.v1 = packet
            return self.scheduler.hold_current()
        if self.v1 is None:
            return self.scheduler.suspend_current()
        v = self.v1
        self.v1 = None
        return self.scheduler.queue(v)


class WorkerTask:
    def __init__(self, scheduler):
        self.scheduler = scheduler
        self.v1 = ID_HANDLER_A
        self.v2 = 0

    def run(self, packet):
        if packet is None:
            return self.scheduler.suspend_current()
        if self.v1 == ID_HANDLER_A:
            self.v1 = ID_HANDLER_B
        else:
            self.v1 = ID_HANDLER_A
        packet.id = self.v1
        packet.a1 = 0
        for i in range(DATA_SIZE):
            self.v2 += 1
            if self.v2 > 26:
                self.v2 = 1
            packet.a2[i] = self.v2
        return self.scheduler.queue(packet)


class HandlerTask:
    def __init__(self, scheduler):
        self.scheduler = scheduler
        self.v1 = None
        self.v2 = None

    def run(self, packet):
        if packet is not None:
            if packet.kind == KIND_WORK:
                self.v1 = packet.add_to(self.v1)
            else:
                self.v2 = packet.add_to(self.v2)
        if self.v1 is not None:
            count = self.v1.a1
            if count < DATA_SIZE:
                if self.v2 is not None:
                    v = self.v2
                    self.v2 = self.v2.link
                    v.a1 = self.v1.a2[count]
                    self.v1.a1 = count + 1
                    return self.scheduler.queue(v)
            else:
                v = self.v1
                self.v1 = self.v1.link
                return self.scheduler.queue(v)
        return self.scheduler.suspend_current()


class Scheduler:
    def __init__(self):
        self.queue_count = 0
        self.hold_count = 0
        self.blocks = [None] * 6
        self.list = None
        self.current_tcb = None
        self.current_id = None

    def add_task(self, id, priority, queue, task):
        self.current_tcb = Tcb(self.list, id, priority, queue, task)
        self.list = self.current_tcb
        self.blocks[id] = self.current_tcb

    def add_running_task(self, id, priority, queue, task):
        self.add_task(id, priority, queue, task)
        self.current_tcb.set_running()

    def add_idle_task(self, id, priority, queue, count):
        self.add_running_task(id, priority, queue, IdleTask(self, 1, count))

    def add_worker_task(self, id, priority, queue):
        self.add_task(id, priority, queue, WorkerTask(self))

    def add_handler_task(self, id, priority, queue):
        self.add_task(id, priority, queue, HandlerTask(self))

    def add_device_task(self, id, priority, queue):
        self.add_task(id, priority, queue, DeviceTask(self))

    def schedule(self):
        self.current_tcb = self.list
        while self.current_tcb is not None:
            if self.current_tcb.is_held_or_suspended():
                self.current_tcb = self.current_tcb.link
            else:
                self.current_id = self.current_tcb.id
                self.current_tcb = self.current_tcb.run()

    def release(self, id):
        tcb = self.blocks[id]
        tcb.mark_as_not_held()
        if tcb.priority > self.current_tcb.priority:
            return tcb
        return self.current_tcb

    def hold_current(self):
        self.hold_count += 1
        self.current_tcb.mark_as_held()
        return self.current_tcb.link

    def suspend_current(self):
        self.current_tcb.mark_as_suspended()
        return self.current_tcb

    def queue(self, packet):
        t = self.blocks[packet.id]
        if t is None:
            return None
        self.queue_count += 1
        packet.link = None
        packet.id = self.current_id
        return t.check_priority_add(self.current_tcb, packet)


rounds = 50
q_total = 0
h_total = 0
for k in range(rounds):
    s = Scheduler()

    s.add_idle_task(ID_IDLE, 0, None, 1000)

    q = Packet(None, ID_WORKER, KIND_WORK)
    q = Packet(q, ID_WORKER, KIND_WORK)
    s.add_worker_task(ID_WORKER, 1000, q)

    q = Packet(None, ID_DEVICE_A, KIND_DEVICE)
    q = Packet(q, ID_DEVICE_A, KIND_DEVICE)
    q = Packet(q, ID_DEVICE_A, KIND_DEVICE)
    s.add_handler_task(ID_HANDLER_A, 2000, q)

    q = Packet(None, ID_DEVICE_B, KIND_DEVICE)
    q = Packet(q, ID_DEVICE_B, KIND_DEVICE)
    q = Packet(q, ID_DEVICE_B, KIND_DEVICE)
    s.add_handler_task(ID_HANDLER_B, 3000, q)

    s.add_device_task(ID_DEVICE_A, 4000, None)
    s.add_device_task(ID_DEVICE_B, 5000, None)

    s.schedule()

    q_total += s.queue_count
    h_total += s.hold_count

if q_total == rounds * 2322 and h_total == rounds * 928:
    print('richards: ok')
else:
    print('richards: FAIL q=' + str(q_total) + ' h=' + str(h_total))
