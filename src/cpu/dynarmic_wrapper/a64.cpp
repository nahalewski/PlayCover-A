/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
// EXPERIMENTAL: AArch64 (A64) CPU wrapper around dynarmic's A64 frontend.
//
// This is a research prototype for 64-bit guest support and is completely
// separate from the 32-bit wrapper in lib.cpp. It does NOT use touchHLE's
// `Mem` (which is a 4GiB, 32-bit-addressed array); instead it operates on a
// sparse owned host buffers mapped at arbitrary 64-bit guest addresses.
// All memory accesses go through bounds/permission-checked callbacks (no page
// table / fastmem), so it is slow but simple.
//
// Only compiled when the `a64` cargo feature of touchHLE_dynarmic_wrapper is
// enabled.
#include <array>
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <optional>
#include <limits>
#include <vector>
#include <chrono>

#include "dynarmic/interface/A64/a64.h"
#include "dynarmic/interface/A64/config.h"
#include "dynarmic/interface/exclusive_monitor.h"

namespace touchHLE::cpu_a64 {

using VAddr = std::uint64_t;
using Vector = Dynarmic::A64::Vector;

const auto HaltReasonSvc = Dynarmic::HaltReason::UserDefined1;
const auto HaltReasonUndefinedInstruction = Dynarmic::HaltReason::UserDefined2;
const auto HaltReasonBreakpoint = Dynarmic::HaltReason::UserDefined3;

class A64Environment final : public Dynarmic::A64::UserCallbacks {
public:
  Dynarmic::A64::Jit *cpu = nullptr;
  struct Region {
    VAddr base;
    std::size_t size;
    std::uint8_t *buf;
    std::uint32_t permissions;
    std::uint32_t max_permissions;
  };
  std::vector<Region> regions;
  std::uint64_t ticks_remaining = 0;
  std::uint32_t halting_svc = 0;
  bool mem_error = false;
  VAddr mem_error_addr = 0;
  // One virtual-kernel epoch per CPU, independent of saved thread registers.
  const std::chrono::steady_clock::time_point clock_epoch = std::chrono::steady_clock::now();

private:
  const Region *region_at(VAddr vaddr) const {
    auto it = std::upper_bound(regions.begin(), regions.end(), vaddr,
                               [](VAddr address, const Region &region) {
                                 return address < region.base;
                               });
    if (it == regions.begin()) return nullptr;
    --it;
    return vaddr - it->base < it->size ? &*it : nullptr;
  }

  // Validate every piece before reading or writing any bytes, including vectors.
  // Adjacent regions may participate; gaps and permission failures stop accesses.
  bool validate(VAddr vaddr, std::size_t size, std::uint32_t permissions,
                VAddr &bad_address) const {
    bad_address = vaddr;
    if (size > std::numeric_limits<VAddr>::max() - vaddr) return false;
    while (size != 0) {
      bad_address = vaddr;
      const auto *region = region_at(vaddr);
      if (!region || (region->permissions & permissions) != permissions)
        return false;
      const auto length = std::min(size, region->size - std::size_t(vaddr - region->base));
      vaddr += length;
      size -= length;
    }
    return true;
  }

  void copy_from_guest(VAddr vaddr, void *destination, std::size_t size) const {
    auto *out = static_cast<std::uint8_t *>(destination);
    while (size != 0) {
      const auto *region = region_at(vaddr);
      const auto offset = std::size_t(vaddr - region->base);
      const auto length = std::min(size, region->size - offset);
      std::memcpy(out, region->buf + offset, length);
      out += length;
      vaddr += length;
      size -= length;
    }
  }

  void copy_to_guest(VAddr vaddr, const void *source, std::size_t size) {
    const auto *in = static_cast<const std::uint8_t *>(source);
    while (size != 0) {
      const auto *region = region_at(vaddr);
      const auto offset = std::size_t(vaddr - region->base);
      const auto length = std::min(size, region->size - offset);
      std::memcpy(region->buf + offset, in, length);
      in += length;
      vaddr += length;
      size -= length;
    }
  }

  void fault(VAddr vaddr) {
    if (!mem_error) {
      mem_error = true;
      mem_error_addr = vaddr;
    }
    cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
  }

  template <typename T> T read(VAddr vaddr) {
    VAddr bad_address;
    if (!validate(vaddr, sizeof(T), 1, bad_address)) {
      fault(bad_address);
      return T{};
    }
    T value;
    copy_from_guest(vaddr, &value, sizeof(T));
    return value;
  }
  template <typename T> void write(VAddr vaddr, T value) {
    VAddr bad_address;
    if (!validate(vaddr, sizeof(T), 2, bad_address)) {
      fault(bad_address);
      return;
    }
    copy_to_guest(vaddr, &value, sizeof(T));
  }
  // Single-threaded host, so a plain compare-and-write is sufficient here.
  template <typename T> bool write_exclusive(VAddr vaddr, T value, T expected) {
    VAddr bad_address;
    if (!validate(vaddr, sizeof(T), 3, bad_address)) {
      fault(bad_address);
      return false;
    }
    T current;
    copy_from_guest(vaddr, &current, sizeof(T));
    if (current != expected) {
      return false;
    }
    copy_to_guest(vaddr, &value, sizeof(T));
    return true;
  }

public:
  bool map(std::uint8_t *buf, std::size_t size, VAddr base,
           std::uint32_t permissions, std::uint32_t max_permissions) {
    if (!buf || size == 0 || permissions > 7 || max_permissions > 7 ||
        (permissions & max_permissions) != permissions ||
        size > std::numeric_limits<VAddr>::max() - base) return false;
    const auto end = base + size;
    auto it = std::lower_bound(regions.begin(), regions.end(), base,
                              [](const Region &region, VAddr address) {
                                return region.base < address;
                              });
    if (it != regions.end() && it->base < end) return false;
    if (it != regions.begin()) {
      const auto &previous = *(it - 1);
      if (previous.base + previous.size > base) return false;
    }
    try {
      regions.insert(it, Region{base, size, buf, permissions, max_permissions});
    } catch (const std::bad_alloc &) {
      return false;
    }
    return true;
  }

  bool unmap(VAddr base, std::size_t size) {
    if (!size || size > std::numeric_limits<VAddr>::max()-base) return false;
    const auto end=base+size;
    VAddr cursor=base;
    while(cursor<end) {const auto *r=region_at(cursor);if(!r)return false;cursor=std::min(end,r->base+r->size);}
    if(regions.size()>65534)return false;
    std::vector<Region> changed;
    try {changed.reserve(regions.size()+1);
      for(const auto &r:regions) {
        const auto stop=r.base+r.size;
        if(stop<=base || r.base>=end){changed.push_back(r);continue;}
        if(r.base<base)changed.push_back(Region{r.base,std::size_t(base-r.base),r.buf,r.permissions,r.max_permissions});
        if(stop>end)changed.push_back(Region{end,std::size_t(stop-end),r.buf+std::size_t(end-r.base),r.permissions,r.max_permissions});
      }
    }catch(const std::bad_alloc &){return false;}
    regions.swap(changed);return true;
  }

  bool protect(VAddr base, std::size_t size, std::uint32_t permissions) {
    if (size == 0 || permissions > 7 || size > std::numeric_limits<VAddr>::max()-base)
      return false;
    const auto end=base+size;
    VAddr cursor=base;
    while (cursor<end) {
      const auto *region=region_at(cursor);
      if (!region || (permissions & region->max_permissions)!=permissions) return false;
      cursor=std::min(end,region->base+region->size);
    }
    if (regions.size()>65534) return false;
    std::vector<Region> changed;
    try {
      changed.reserve(regions.size()+2);
      for (const auto &region:regions) {
        const auto region_end=region.base+region.size;
        if (region_end<=base || region.base>=end) {changed.push_back(region);continue;}
        const auto start=std::max(region.base,base), stop=std::min(region_end,end);
        if (region.base<start)
          changed.push_back(Region{region.base,std::size_t(start-region.base),region.buf,region.permissions,region.max_permissions});
        changed.push_back(Region{start,std::size_t(stop-start),region.buf+std::size_t(start-region.base),permissions,region.max_permissions});
        if (stop<region_end)
          changed.push_back(Region{stop,std::size_t(region_end-stop),region.buf+std::size_t(stop-region.base),region.permissions,region.max_permissions});
      }
    } catch (const std::bad_alloc &) {return false;}
    regions.swap(changed);
    return true;
  }

  std::optional<std::uint32_t> MemoryReadCode(VAddr vaddr) override {
    VAddr bad_address;
    if (!validate(vaddr, 4, 4, bad_address)) {
      return std::nullopt;
    }
    std::uint32_t value;
    copy_from_guest(vaddr, &value, 4);
    return value;
  }

  std::uint8_t MemoryRead8(VAddr vaddr) override {
    return read<std::uint8_t>(vaddr);
  }
  std::uint16_t MemoryRead16(VAddr vaddr) override {
    return read<std::uint16_t>(vaddr);
  }
  std::uint32_t MemoryRead32(VAddr vaddr) override {
    return read<std::uint32_t>(vaddr);
  }
  std::uint64_t MemoryRead64(VAddr vaddr) override {
    return read<std::uint64_t>(vaddr);
  }
  Vector MemoryRead128(VAddr vaddr) override {
    return read<Vector>(vaddr);
  }

  void MemoryWrite8(VAddr vaddr, std::uint8_t value) override {
    write(vaddr, value);
  }
  void MemoryWrite16(VAddr vaddr, std::uint16_t value) override {
    write(vaddr, value);
  }
  void MemoryWrite32(VAddr vaddr, std::uint32_t value) override {
    write(vaddr, value);
  }
  void MemoryWrite64(VAddr vaddr, std::uint64_t value) override {
    write(vaddr, value);
  }
  void MemoryWrite128(VAddr vaddr, Vector value) override {
    write<Vector>(vaddr, value);
  }

  bool MemoryWriteExclusive8(VAddr vaddr, std::uint8_t value,
                             std::uint8_t expected) override {
    return write_exclusive(vaddr, value, expected);
  }
  bool MemoryWriteExclusive16(VAddr vaddr, std::uint16_t value,
                              std::uint16_t expected) override {
    return write_exclusive(vaddr, value, expected);
  }
  bool MemoryWriteExclusive32(VAddr vaddr, std::uint32_t value,
                              std::uint32_t expected) override {
    return write_exclusive(vaddr, value, expected);
  }
  bool MemoryWriteExclusive64(VAddr vaddr, std::uint64_t value,
                              std::uint64_t expected) override {
    return write_exclusive(vaddr, value, expected);
  }
  bool MemoryWriteExclusive128(VAddr vaddr, Vector value,
                               Vector expected) override {
    return write_exclusive(vaddr, value, expected);
  }

  void InterpreterFallback(VAddr pc, std::size_t num_instructions) override {
    (void)num_instructions;
    VAddr bad_address;
    if (!validate(pc, 4, 4, bad_address)) {
      fault(bad_address);
    } else {
      // This prototype has no interpreter, so report unsupported guest code
      // through the normal halt channel rather than terminating the host.
      cpu->HaltExecution(HaltReasonUndefinedInstruction);
    }
  }
  void CallSVC(std::uint32_t svc) override {
    halting_svc = svc;
    cpu->HaltExecution(HaltReasonSvc);
  }
  void ExceptionRaised(VAddr pc, Dynarmic::A64::Exception exception) override {
    using Exception = Dynarmic::A64::Exception;
    switch (exception) {
    case Exception::NoExecuteFault:
      fault(pc);
      break;
    case Exception::Breakpoint:
      cpu->HaltExecution(HaltReasonBreakpoint);
      break;
    case Exception::UnallocatedEncoding:
    case Exception::ReservedValue:
    case Exception::UnpredictableInstruction:
      cpu->HaltExecution(HaltReasonUndefinedInstruction);
      break;
    // Hint instructions: nothing to do for a single-core user-mode emulator.
    case Exception::WaitForInterrupt:
    case Exception::WaitForEvent:
    case Exception::SendEvent:
    case Exception::SendEventLocal:
    case Exception::Yield:
      break;
    default:
      cpu->HaltExecution(HaltReasonUndefinedInstruction);
      break;
    }
  }
  void AddTicks(std::uint64_t ticks) override {
    ticks_remaining = ticks > ticks_remaining ? 0 : ticks_remaining - ticks;
  }
  std::uint64_t GetTicksRemaining() override { return ticks_remaining; }
  std::uint64_t GetCNTPCT() override {
    const auto elapsed=std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now()-clock_epoch).count();
    return elapsed>0 ? std::uint64_t(elapsed) : 0;
  }
};

class A64Wrapper {
  A64Environment env;
  std::unique_ptr<Dynarmic::ExclusiveMonitor> mon;
  std::unique_ptr<Dynarmic::A64::Jit> cpu;
  // iOS arm64 uses TPIDRRO_EL0 for the thread pointer.
  std::uint64_t tpidrro_el0 = 0;
  std::uint64_t tpidr_el0 = 0;

public:
  // This vendored A64 frontend has no Dynarmic Context API. Keep the complete
  // guest-visible register snapshot opaque to Rust instead of exposing layout.
  struct Context {
    std::array<std::uint64_t, 31> registers;
    std::array<Vector, 32> vectors;
    std::uint64_t pc;
    std::uint64_t sp;
    std::uint32_t pstate;
    std::uint32_t fpcr;
    std::uint32_t fpsr;
    std::uint64_t tpidrro_el0;
    std::uint64_t tpidr_el0;
  };

  A64Wrapper(std::uint8_t *buf, std::size_t len, VAddr base) {
    if (len != 0 && !env.map(buf, len, base, 7, 7)) std::abort();
    Dynarmic::A64::UserConfig config;
    config.callbacks = &env;
    mon = std::make_unique<Dynarmic::ExclusiveMonitor>(1);
    config.global_monitor = mon.get();
    config.tpidrro_el0 = &tpidrro_el0;
    config.tpidr_el0 = &tpidr_el0;
    // Counter ticks are actual steady-clock nanoseconds. No Apple hardware
    // counter frequency is impersonated; CNTVCT translation is not advertised.
    config.cntfrq_el0 = 1000000000;
    config.wall_clock_cntpct = true;
    // page_table stays nullptr: every access goes through the callbacks.
    cpu = std::make_unique<Dynarmic::A64::Jit>(config);
    env.cpu = cpu.get();
  }

  std::uint64_t get_reg(std::size_t idx) const { return cpu->GetRegister(idx); }
  std::uint64_t counter_ticks() {return env.GetCNTPCT();}
  std::uint32_t counter_frequency() const {return 1000000000;}
  bool map(std::uint8_t *buf, std::size_t size, VAddr base, std::uint32_t permissions) {
    return map_with_max(buf,size,base,permissions,permissions);
  }
  bool map_with_max(std::uint8_t *buf,std::size_t size,VAddr base,std::uint32_t permissions,std::uint32_t max_permissions) {
    if (!env.map(buf, size, base, permissions,max_permissions)) return false;
    // A previously translated no-execute fault at this address is now obsolete.
    cpu->InvalidateCacheRange(base, size);
    return true;
  }
  bool protect(VAddr base,std::size_t size,std::uint32_t permissions) {
    if (!env.protect(base,size,permissions)) return false;
    cpu->InvalidateCacheRange(base,size);
    return true;
  }
  bool unmap(VAddr base,std::size_t size) {
    if(!env.unmap(base,size))return false;
    cpu->ClearExclusiveState();cpu->InvalidateCacheRange(base,size);return true;
  }
  void set_reg(std::size_t idx, std::uint64_t v) { cpu->SetRegister(idx, v); }
  void get_vector(std::size_t idx, std::uint64_t *lanes) const {
    const auto vector = cpu->GetVectors().at(idx);
    lanes[0] = vector[0];
    lanes[1] = vector[1];
  }
  void set_vector(std::size_t idx, const std::uint64_t *lanes) {
    auto vectors = cpu->GetVectors();
    vectors.at(idx) = {lanes[0], lanes[1]};
    cpu->SetVectors(vectors);
  }
  std::uint64_t get_pc() const { return cpu->GetPC(); }
  void set_pc(std::uint64_t v) { cpu->SetPC(v); }
  std::uint64_t get_sp() const { return cpu->GetSP(); }
  void set_sp(std::uint64_t v) { cpu->SetSP(v); }
  std::uint32_t get_pstate() const { return cpu->GetPstate(); }
  void set_pstate(std::uint32_t value) { cpu->SetPstate(value); }
  void set_tpidrro_el0(std::uint64_t v) { tpidrro_el0 = v; }
  std::uint64_t get_tpidrro_el0() const { return tpidrro_el0; }
  std::uint64_t get_tpidr_el0() const { return tpidr_el0; }
  void set_tpidr_el0(std::uint64_t v) { tpidr_el0 = v; }
  std::uint64_t mem_error_addr() const { return env.mem_error_addr; }

  void save_context(Context &context) const {
    context = {cpu->GetRegisters(), cpu->GetVectors(), cpu->GetPC(),
               cpu->GetSP(), cpu->GetPstate(), cpu->GetFpcr(), cpu->GetFpsr(),
               tpidrro_el0, tpidr_el0};
  }

  void restore_context(const Context &context) {
    cpu->SetRegisters(context.registers);
    cpu->SetVectors(context.vectors);
    cpu->SetPC(context.pc);
    cpu->SetSP(context.sp);
    cpu->SetPstate(context.pstate);
    cpu->SetFpcr(context.fpcr);
    cpu->SetFpsr(context.fpsr);
    tpidrro_el0 = context.tpidrro_el0;
    tpidr_el0 = context.tpidr_el0;
    // Reservations cannot cross a host-driven guest function call boundary.
    // The A64 API does not expose their internal state for snapshotting.
    cpu->ClearExclusiveState();
  }

  void invalidate_cache_range(VAddr start, std::size_t size) {
    cpu->InvalidateCacheRange(start, size);
  }

  // Same result convention as DynarmicWrapper::run_or_step in lib.cpp:
  // -1 normal (ticks exhausted / step done), -2 memory error,
  // -3 undefined instruction, -4 breakpoint, >= 0 SVC immediate.
  std::int32_t run_or_step(std::uint64_t *ticks) {
    env.mem_error = false;
    Dynarmic::HaltReason hr;
    if (ticks) {
      env.ticks_remaining = *ticks;
      hr = cpu->Run();
    } else {
      hr = cpu->Step();
    }
    std::int32_t res;
    if (env.mem_error || Dynarmic::Has(hr, Dynarmic::HaltReason::MemoryAbort)) {
      res = -2;
    } else if (Dynarmic::Has(hr, HaltReasonUndefinedInstruction)) {
      res = -3;
    } else if (Dynarmic::Has(hr, HaltReasonBreakpoint)) {
      res = -4;
    } else if (Dynarmic::Has(hr, HaltReasonSvc)) {
      res = std::int32_t(halting_svc_mask(env.halting_svc));
    } else if ((!hr && ticks) || (hr == Dynarmic::HaltReason::Step && !ticks)) {
      res = -1;
    } else {
      std::fprintf(stderr, "A64: unhandled halt reason %u\n", unsigned(hr));
      std::abort();
    }
    if (ticks) {
      *ticks = env.ticks_remaining;
    }
    return res;
  }

private:
  static std::uint32_t halting_svc_mask(std::uint32_t svc) {
    return svc & 0xffff; // A64 SVC immediate is 16 bits
  }
};

extern "C" {

A64Wrapper *touchHLE_A64Wrapper_new(std::uint8_t *buf, std::size_t len,
                                    std::uint64_t base) {
  return new A64Wrapper(buf, len, base);
}
void touchHLE_A64Wrapper_delete(A64Wrapper *cpu) { delete cpu; }
std::uint64_t touchHLE_A64Wrapper_counter_ticks(A64Wrapper *cpu) {return cpu->counter_ticks();}
std::uint32_t touchHLE_A64Wrapper_counter_frequency(const A64Wrapper *cpu) {return cpu->counter_frequency();}
bool touchHLE_A64Wrapper_map(A64Wrapper *cpu, std::uint8_t *buf,
                            std::size_t size, std::uint64_t base,
                            std::uint32_t permissions) {
  return cpu->map(buf, size, base, permissions);
}
bool touchHLE_A64Wrapper_map_with_max(A64Wrapper *cpu,std::uint8_t *buf,std::size_t size,std::uint64_t base,std::uint32_t permissions,std::uint32_t max_permissions) {
  return cpu->map_with_max(buf,size,base,permissions,max_permissions);
}
bool touchHLE_A64Wrapper_protect(A64Wrapper *cpu,std::uint64_t base,std::size_t size,std::uint32_t permissions) {
  return cpu->protect(base,size,permissions);
}
bool touchHLE_A64Wrapper_unmap(A64Wrapper *cpu,std::uint64_t base,std::size_t size) {
  return cpu->unmap(base,size);
}
A64Wrapper::Context *touchHLE_A64Context_new() {
  return new A64Wrapper::Context{};
}
void touchHLE_A64Context_delete(A64Wrapper::Context *context) { delete context; }
void touchHLE_A64Wrapper_save_context(const A64Wrapper *cpu,
                                      A64Wrapper::Context *context) {
  cpu->save_context(*context);
}
void touchHLE_A64Wrapper_restore_context(A64Wrapper *cpu,
                                         const A64Wrapper::Context *context) {
  cpu->restore_context(*context);
}
std::uint64_t touchHLE_A64Wrapper_get_reg(const A64Wrapper *cpu,
                                          std::size_t idx) {
  return cpu->get_reg(idx);
}
void touchHLE_A64Wrapper_set_reg(A64Wrapper *cpu, std::size_t idx,
                                 std::uint64_t v) {
  cpu->set_reg(idx, v);
}
void touchHLE_A64Wrapper_get_vector(const A64Wrapper *cpu, std::size_t idx,
                                    std::uint64_t *lanes) {
  cpu->get_vector(idx, lanes);
}
void touchHLE_A64Wrapper_set_vector(A64Wrapper *cpu, std::size_t idx,
                                    const std::uint64_t *lanes) {
  cpu->set_vector(idx, lanes);
}
std::uint64_t touchHLE_A64Wrapper_get_pc(const A64Wrapper *cpu) {
  return cpu->get_pc();
}
void touchHLE_A64Wrapper_set_pc(A64Wrapper *cpu, std::uint64_t v) {
  cpu->set_pc(v);
}
std::uint64_t touchHLE_A64Wrapper_get_sp(const A64Wrapper *cpu) {
  return cpu->get_sp();
}
void touchHLE_A64Wrapper_set_sp(A64Wrapper *cpu, std::uint64_t v) {
  cpu->set_sp(v);
}
std::uint32_t touchHLE_A64Wrapper_get_pstate(const A64Wrapper *cpu) {
  return cpu->get_pstate();
}
void touchHLE_A64Wrapper_set_pstate(A64Wrapper *cpu, std::uint32_t value) {
  cpu->set_pstate(value);
}
void touchHLE_A64Wrapper_set_tpidrro_el0(A64Wrapper *cpu, std::uint64_t v) {
  cpu->set_tpidrro_el0(v);
}
std::uint64_t touchHLE_A64Wrapper_get_tpidrro_el0(const A64Wrapper *cpu) {
  return cpu->get_tpidrro_el0();
}
std::uint64_t touchHLE_A64Wrapper_get_tpidr_el0(const A64Wrapper *cpu) {
  return cpu->get_tpidr_el0();
}
void touchHLE_A64Wrapper_set_tpidr_el0(A64Wrapper *cpu, std::uint64_t v) {
  cpu->set_tpidr_el0(v);
}
std::uint64_t touchHLE_A64Wrapper_mem_error_addr(const A64Wrapper *cpu) {
  return cpu->mem_error_addr();
}
void touchHLE_A64Wrapper_invalidate_cache_range(A64Wrapper *cpu,
                                                std::uint64_t start,
                                                std::size_t size) {
  cpu->invalidate_cache_range(start, size);
}
std::int32_t touchHLE_A64Wrapper_run_or_step(A64Wrapper *cpu,
                                             std::uint64_t *ticks) {
  return cpu->run_or_step(ticks);
}
}

} // namespace touchHLE::cpu_a64
