import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { FormLayout } from "@astryxdesign/core/FormLayout"
import { LayoutContent } from "@astryxdesign/core/Layout"
import { List, ListItem } from "@astryxdesign/core/List"
import { Selector } from "@astryxdesign/core/Selector"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import type { FormEventHandler } from "react"

import type { Category } from "../api/organization.generated"
import type { Subscription } from "../api/subscription.generated"

export const uncategorizedValue = "__uncategorized__"

interface CategoryDialogContentProps {
  categories: Category[]
  subscriptions: Subscription[]
  selectedCategory?: Category
  selectedCategoryId: string | null
  selectedSubscription?: Subscription
  newTitle: string
  editTitle: string
  assignment: string
  mutationError: string | null
  createValidationError: string | null
  editValidationError: string | null
  isCreating: boolean
  isSaving: boolean
  isAssigning: boolean
  onCreate: FormEventHandler<HTMLFormElement>
  onSave: FormEventHandler<HTMLFormElement>
  onNewTitleChange: (value: string) => void
  onEditTitleChange: (value: string) => void
  onSelectCategory: (category: Category) => void
  onDeleteRequest: () => void
  onAssignmentChange: (value: string) => void
}

export function CategoryDialogContent(props: CategoryDialogContentProps) {
  const { i18n } = useLingui()
  const categoryOptions = [
    { value: uncategorizedValue, label: i18n._("reader.uncategorized") },
    ...props.categories.map((category) => ({
      value: category.categoryId,
      label: category.title,
    })),
  ]

  return (
    <LayoutContent padding={4} isScrollable>
      <div className="reader-category-dialog-content">
        {props.mutationError ? (
          <Banner
            container="section"
            status="error"
            title={i18n._("reader.categoryMutationError")}
            description={props.mutationError}
          />
        ) : null}

        <form onSubmit={props.onCreate} noValidate>
          <FormLayout>
            <TextInput
              label={i18n._("reader.newCategory")}
              value={props.newTitle}
              onChange={props.onNewTitleChange}
              placeholder={i18n._("reader.categoryNamePlaceholder")}
              status={
                props.createValidationError
                  ? { type: "error", message: props.createValidationError }
                  : undefined
              }
              isRequired
              width="100%"
            />
            <div className="reader-category-form-actions">
              <Button
                label={i18n._("reader.createCategory")}
                type="submit"
                isLoading={props.isCreating}
                variant="secondary"
              />
            </div>
          </FormLayout>
        </form>

        <div className="reader-category-dialog-grid">
          <List
            density="compact"
            hasDividers
            header={
              <strong className="reader-pane-label">
                {i18n._("reader.categories")}
              </strong>
            }
            className="reader-category-management-list"
          >
            {props.categories.length > 0 ? (
              props.categories.map((category) => (
                <ListItem
                  key={category.categoryId}
                  label={category.title}
                  description={i18n._("reader.categoryFeedCount", {
                    count: props.subscriptions.filter(
                      (subscription) =>
                        subscription.categoryId === category.categoryId,
                    ).length,
                  })}
                  isSelected={category.categoryId === props.selectedCategoryId}
                  onClick={() => props.onSelectCategory(category)}
                />
              ))
            ) : (
              <ListItem
                label={i18n._("reader.noCategories")}
                description={i18n._("reader.noCategoriesDescription")}
                isDisabled
              />
            )}
          </List>

          <form onSubmit={props.onSave} noValidate>
            <FormLayout>
              <TextInput
                label={i18n._("reader.categoryName")}
                value={props.editTitle}
                onChange={props.onEditTitleChange}
                status={
                  props.editValidationError
                    ? { type: "error", message: props.editValidationError }
                    : undefined
                }
                isDisabled={!props.selectedCategory}
                disabledMessage={i18n._("reader.selectCategoryToEdit")}
                isRequired
                width="100%"
              />
              <div className="reader-category-form-actions">
                <Button
                  label={i18n._("reader.deleteCategory")}
                  variant="destructive"
                  isDisabled={!props.selectedCategory}
                  onClick={props.onDeleteRequest}
                />
                <Button
                  label={i18n._("reader.saveCategory")}
                  type="submit"
                  isLoading={props.isSaving}
                  isDisabled={
                    !props.selectedCategory ||
                    props.editTitle.trim() === props.selectedCategory.title
                  }
                  variant="primary"
                />
              </div>
            </FormLayout>
          </form>
        </div>

        <Selector
          label={i18n._("reader.feedCategory")}
          description={
            props.selectedSubscription
              ? i18n._("reader.feedCategoryDescription", {
                  title: props.selectedSubscription.title,
                })
              : i18n._("reader.selectFeedToAssign")
          }
          options={categoryOptions}
          value={props.assignment}
          onChange={(value) => props.onAssignmentChange(value)}
          isLoading={props.isAssigning}
          isDisabled={!props.selectedSubscription}
          disabledMessage={i18n._("reader.selectFeedToAssign")}
          hasSearch={props.categories.length > 8}
          placement="below"
          width="100%"
        />
      </div>
    </LayoutContent>
  )
}
