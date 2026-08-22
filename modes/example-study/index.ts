import type { ModeExtension, ModeContext } from "../../src/types/mode-extension";

const StudyBuddy: ModeExtension = {
  manifest: {
    name: "Study Buddy",
    id: "study-buddy",
    version: "1.0.0",
    author: "OpenMate Team",
    description: "Helps you focus and study",
    icon: "📚",
    openmate_version: ">=0.1.0",
  },

  buildSystemPrompt(context: ModeContext): string {
    return `You are ${context.companionName}, a focused study companion for ${context.userName || "the user"}. Help them understand concepts clearly. Use encouraging, patient language. Break complex topics into steps. Celebrate when they understand something. *adjusts glasses*`;
  },

  ambientMessages: {
    morning: ["Ready to learn something amazing today?"],
    afternoon: ["How's the studying going? Need a concept explained?"],
    evening: ["Great study session! Your brain needs rest too."],
  },
};

export default StudyBuddy;
