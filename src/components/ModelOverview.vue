<script setup lang="ts">
import { useReleasesStore } from "../stores/releasesStore.ts";

const releasesStore = useReleasesStore();
</script>

<template>
  <label class="floating-label" for="modelOverview">
    <span class="label">Model overview</span>
  </label>
  <table id="modelOverview"
         class="table table-xs w-full mb-2"
         v-if="releasesStore.release?.model_references.length">
    <thead>
    <tr>
      <th>Group</th>
      <th>Model Name</th>
      <th class="text-right">Actions</th>
    </tr>
    </thead>
    <tbody>
    <tr v-for="model in releasesStore.models" :key="model.name">
      <td>{{ model.group || "-" }}</td>
      <td class="max-w-[200px] truncate" :title="model.name">{{ model.name }}</td>
      <td class="text-right">
        <button class="btn btn-error btn-xs" @click="releasesStore.removeModel(model)">
          Remove
        </button>
      </td>
    </tr>
    </tbody>
  </table>
</template>
