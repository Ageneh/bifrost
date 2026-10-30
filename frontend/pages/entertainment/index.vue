<script setup lang="ts">
/**
 * Entertainment: the zones there are, and the three ways to get one.
 *
 * The preflight is above the recipes rather than inside them, because it is
 * about the bridge and not about a recipe: when it is red, every recipe is
 * blocked for the same reason.
 */
import { computed, onMounted, ref } from 'vue'
import AsyncState from '~/components/AsyncState.vue'
import EntertainmentWizard from '~/components/EntertainmentWizard.vue'
import PreflightChecklist from '~/components/PreflightChecklist.vue'
import RecipeCard from '~/components/RecipeCard.vue'
import { useEntertainment } from '~/composables/useEntertainment'
import { useToast } from '~/composables/useToast'

const entertainment = useEntertainment()
const toast = useToast()

/** The recipe the wizard is carrying out, or none. */
const recipe = ref<Recipe | null>(null)

interface Recipe {
  key: string
  title: string
  why: string
  suggestion: string
  requirements: string[]
  pairing?: boolean
}

const RECIPES: Recipe[] = [
  {
    key: 'ambilight',
    title: 'Add a TV with Ambilight',
    why: 'A screen zone whose lights follow the picture. The Hue app creates the same zone; this does it without leaving the console.',
    suggestion: 'Living room TV',
    requirements: [
      'A TV with Ambilight+Hue, or the Hue Play gradient lights behind it',
      'The lights in the zone already paired to this bridge',
    ],
  },
  {
    key: 'sync-box',
    title: 'Add a Sync box',
    why: 'A screen zone for the HDMI sources on a Sync box, plus the pairing step the box needs before it will stream.',
    suggestion: 'Sync box',
    requirements: [
      'The lights the box should drive',
      'The box paired to this bridge afterwards',
    ],
    pairing: true,
  },
  {
    key: 'second-zone',
    title: 'Add a second zone',
    why: 'A second screen for a monitor or another TV. Two zones stream at once, which is what bifrost.entertainment.max_streams allows by default.',
    suggestion: 'Desk monitor',
    requirements: [
      'Lights that are not in another zone: a light belongs to one',
      'A second client, if both zones should stream together',
    ],
  },
]

/** Why every recipe is blocked, when the bridge cannot stream at all. */
const blockedReason = computed(() => {
  const red = entertainment.preflight.value.find(row => row.state === 'crit')
  return red === undefined ? null : red.detail
})

const available = computed(
  () => entertainment.ready.value && entertainment.devices.value.length > 0,
)

function start(key: string): void {
  recipe.value = RECIPES.find(entry => entry.key === key) ?? null
}

async function created(id: string): Promise<void> {
  const pairing = recipe.value?.pairing === true
  recipe.value = null
  await entertainment.load()
  toast.ok(pairing ? 'Zone created. Pair the Sync box next.' : 'Zone created.')
  await navigateTo(`/entertainment/${id}`)
}

onMounted(async () => {
  await entertainment.load()
})
</script>

<template>
  <div
    class="entertainment"
    data-testid="entertainment-home"
  >
    <AsyncState
      :loading="entertainment.loading.value && entertainment.capabilities.value === null"
      :error="entertainment.failure.value"
      :skeleton-rows="4"
      testid="entertainment-state"
      @retry="entertainment.load"
    >
      <header class="fd-between">
        <div>
          <div class="fd-label">
            Entertainment
          </div>
          <h1 class="fd-panel__title">
            Zones, recipes, and what is streaming
          </h1>
          <div class="fd-panel__sub">
            A zone is a set of lights the TV colours. The TV or the Sync app
            starts the stream; this console places the lights and says why one
            is not running.
          </div>
        </div>
      </header>

      <div class="columns">
        <div class="fd-stack">
          <section
            class="fd-panel"
            data-testid="entertainment-preflight"
          >
            <div class="fd-panel__head">
              <div>
                <div class="fd-panel__title">
                  Before a stream can run
                </div>
                <div class="fd-panel__sub">
                  Each row is what the bridge can see. A red row links to its
                  fix.
                </div>
              </div>
            </div>
            <div class="fd-panel__body">
              <PreflightChecklist
                :rows="entertainment.preflight.value"
                testid="entertainment-preflight-list"
              />
            </div>
          </section>

          <section
            class="fd-panel"
            data-testid="entertainment-recipes"
          >
            <div class="fd-panel__head">
              <div>
                <div class="fd-panel__title">
                  Add a zone
                </div>
                <div class="fd-panel__sub">
                  Three ways in, one resource out. Each creates a zone the editor
                  can change afterwards.
                </div>
              </div>
            </div>
            <div class="fd-panel__body recipes">
              <RecipeCard
                v-for="entry in RECIPES"
                :key="entry.key"
                :title="entry.title"
                :why="entry.why"
                :requirements="entry.requirements"
                :available="available"
                :blocked="blockedReason"
                :testid="`recipe-${entry.key}`"
                @start="start(entry.key)"
              />
            </div>
          </section>

          <section
            class="fd-panel"
            data-testid="entertainment-zones"
          >
            <div class="fd-panel__head">
              <div>
                <div class="fd-panel__title">
                  Zones
                </div>
                <div class="fd-panel__sub">
                  {{ entertainment.zones.value.length }} zone{{ entertainment.zones.value.length === 1 ? '' : 's' }}
                  on this bridge.
                </div>
              </div>
            </div>
            <div class="fd-panel__body fd-stack">
              <div
                v-if="entertainment.zones.value.length === 0"
                class="fd-muted"
                data-testid="entertainment-zones-empty"
              >
                No zone yet. Start with a recipe above.
              </div>

              <NuxtLink
                v-for="zone in entertainment.zones.value"
                :key="zone.id"
                :to="`/entertainment/${zone.id}`"
                class="fd-row"
                :data-testid="`zone-${zone.id}`"
                :data-state="zone.status === 'active' ? 'active' : 'idle'"
              >
                <div class="fd-row__top">
                  <span class="fd-row__name">{{ zone.name }}</span>
                  <span class="fd-meta fd-muted">
                    {{ zone.configuration_type }} · {{ zone.channels }} channel{{ zone.channels === 1 ? '' : 's' }}
                  </span>
                </div>
                <div class="fd-row__path">
                  <b>{{ zone.lights.length }}</b> light{{ zone.lights.length === 1 ? '' : 's' }}
                  · {{ zone.status }}
                </div>
              </NuxtLink>
            </div>
          </section>
        </div>
      </div>
    </AsyncState>

    <EntertainmentWizard
      :open="recipe !== null"
      :title="recipe?.title ?? ''"
      :suggestion="recipe?.suggestion ?? ''"
      :pairing="recipe?.pairing"
      @close="recipe = null"
      @created="created"
    />
  </div>
</template>

<style scoped>
.entertainment {
  display: flex;
  flex-direction: column;
  gap: var(--fd-3);
  min-width: 0;
}
.columns {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: var(--fd-3);
  align-items: start;
}
.recipes {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: var(--fd-3);
}
</style>
