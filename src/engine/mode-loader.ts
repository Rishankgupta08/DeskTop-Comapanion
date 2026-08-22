/**
 * OpenMate Mode Extension Loader
 *
 * Discovers, validates, and loads community TypeScript mode extensions. [DR-037]
 *
 * ## Security rules:
 * - Mode ID must strictly match /^[a-z0-9-]{2,32}$/ (prevents path traversal)
 * - Manifest must pass strict schema and SemVer validation
 * - buildSystemPrompt() must return a clean string
 * - handleToolTag() can only return a ToolResponse DTO and cannot call IPC directly
 */

import type {
  ModeExtension,
  ModeManifest,
} from "../types/mode-extension";

// Glob pattern for discovering mode manifests and entrypoints across modes/ directory
const manifestGlobs = import.meta.glob<{ default?: unknown }>(
  ["/modes/*/manifest.json", "../../modes/*/manifest.json"],
  { eager: true }
);

const extensionGlobs = import.meta.glob<{ default?: ModeExtension }>(
  ["/modes/*/index.ts", "../../modes/*/index.ts"]
);

export class ModeLoader {
  private registeredExtensions: Map<string, ModeExtension> = new Map();
  private loadedManifests: Map<string, ModeManifest> = new Map();

  /**
   * Validate mode ID format.
   * Must match `/^[a-z0-9-]{2,32}$/` and must not start or end with a hyphen.
   */
  public validateId(id: string): boolean {
    if (typeof id !== "string") return false;
    const regex = /^[a-z0-9-]{2,32}$/;
    if (!regex.test(id)) return false;
    if (id.startsWith("-") || id.endsWith("-")) return false;
    return true;
  }

  /**
   * Validate manifest structure, required fields, and SemVer version.
   */
  public validateManifest(manifest: unknown): manifest is ModeManifest {
    if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
      return false;
    }

    const m = manifest as Record<string, unknown>;

    // Required non-empty string fields
    const requiredStrings = [
      "name",
      "id",
      "version",
      "author",
      "description",
      "icon",
      "openmate_version",
    ];

    for (const key of requiredStrings) {
      if (typeof m[key] !== "string" || (m[key] as string).trim() === "") {
        return false;
      }
    }

    // ID validation
    if (!this.validateId(m.id as string)) {
      return false;
    }

    // SemVer validation (basic X.Y.Z[-prerelease])
    const semverRegex = /^\d+\.\d+\.\d+(-[a-zA-Z0-9.-]+)?$/;
    if (!semverRegex.test((m.version as string).trim())) {
      return false;
    }

    return true;
  }

  /**
   * Scan `modes/` directory for extension manifests.
   */
  public async scanExtensions(): Promise<ModeManifest[]> {
    const manifests: ModeManifest[] = [];
    const seenIds = new Set<string>();

    // 1. Scan dynamically discovered manifest files via Vite glob
    for (const [path, mod] of Object.entries(manifestGlobs)) {
      try {
        const rawContent = (mod && typeof mod === "object" && "default" in mod)
          ? mod.default
          : mod;

        if (this.validateManifest(rawContent)) {
          if (!seenIds.has(rawContent.id)) {
            seenIds.add(rawContent.id);
            this.loadedManifests.set(rawContent.id, rawContent);
            manifests.push(rawContent);
          }
        } else {
          console.warn(`[ModeLoader] Invalid mode manifest at ${path}`);
        }
      } catch (err) {
        console.warn(`[ModeLoader] Failed to read manifest at ${path}:`, err);
      }
    }

    // 2. Include any in-memory registered extensions
    for (const [id, ext] of this.registeredExtensions.entries()) {
      if (!seenIds.has(id) && this.validateManifest(ext.manifest)) {
        seenIds.add(id);
        manifests.push(ext.manifest);
      }
    }

    return manifests.sort((a, b) => a.name.localeCompare(b.name));
  }

  /**
   * Load and validate a specific mode extension by mode ID.
   */
  public async loadExtension(id: string): Promise<ModeExtension> {
    if (!this.validateId(id)) {
      throw new Error(
        `Invalid mode ID "${id}". Mode IDs must be 2-32 lowercase alphanumeric characters with hyphens.`
      );
    }

    // Check in-memory registered extensions first
    if (this.registeredExtensions.has(id)) {
      const ext = this.registeredExtensions.get(id)!;
      this.validateExtensionObject(ext);
      return ext;
    }

    // Look for matching glob module
    let targetModuleLoader: (() => Promise<{ default?: ModeExtension }>) | null = null;

    for (const [path, loader] of Object.entries(extensionGlobs)) {
      // Check if path contains `/modes/<id>/` or `modes/<id>/`
      const normalized = path.replace(/\\/g, "/");
      if (
        normalized.includes(`/modes/${id}/index.ts`) ||
        normalized.includes(`modes/${id}/index.ts`)
      ) {
        targetModuleLoader = loader;
        break;
      }
    }

    if (!targetModuleLoader) {
      throw new Error(`Mode extension module for "${id}" was not found.`);
    }

    const mod = await targetModuleLoader();
    const ext = mod.default || (mod as unknown as ModeExtension);

    if (!ext || typeof ext !== "object") {
      throw new Error(`Mode extension "${id}" does not export a valid default object.`);
    }

    this.validateExtensionObject(ext);

    if (ext.manifest.id !== id) {
      throw new Error(
        `Mode extension manifest ID "${ext.manifest.id}" does not match requested ID "${id}".`
      );
    }

    this.registeredExtensions.set(id, ext);
    return ext;
  }

  /**
   * Validate that the loaded extension conforms strictly to the ModeExtension interface.
   */
  private validateExtensionObject(ext: ModeExtension): void {
    const rawManifest = ext.manifest as unknown;
    if (!this.validateManifest(rawManifest)) {
      throw new Error(`Invalid manifest in mode extension "${(rawManifest as any)?.id || "unknown"}"`);
    }

    if (typeof ext.buildSystemPrompt !== "function") {
      throw new Error(
        `Mode extension "${ext.manifest.id}" must implement a buildSystemPrompt(context) function.`
      );
    }

    // Validate handleToolTag if provided
    if (ext.handleToolTag !== undefined && typeof ext.handleToolTag !== "function") {
      throw new Error(
        `handleToolTag in mode extension "${ext.manifest.id}" must be a function if defined.`
      );
    }
  }

  /**
   * Register a mode extension directly (useful for testing or built-in registration).
   */
  public registerExtension(extension: ModeExtension): void {
    this.validateExtensionObject(extension);
    this.registeredExtensions.set(extension.manifest.id, extension);
    this.loadedManifests.set(extension.manifest.id, extension.manifest);
  }

  /**
   * Unregister an extension.
   */
  public unregisterExtension(id: string): void {
    this.registeredExtensions.delete(id);
    this.loadedManifests.delete(id);
  }
}

// Global default singleton instance
export const modeLoader = new ModeLoader();
